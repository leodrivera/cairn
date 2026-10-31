#!/usr/bin/env bash
# Runs the README's jail and filter in Ubuntu 24.04's own Fail2ban package (whose defaults are backend = systemd and
# banaction = nftables) inside a container with its own network namespace: eight failed sign-ins written to a
# Docker json-file log must ban the address in the DOCKER-USER chain. The host's firewall is never touched.
set -uo pipefail

source "$(dirname "$0")/lib.sh"
UBUNTU=ubuntu@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852  # 24.04
out=$(docker run --rm --cap-add NET_ADMIN \
  -v "$ROOT/examples/compose/fail2ban/cairn.conf:/etc/fail2ban/filter.d/cairn.conf:ro" \
  -v "$ROOT/examples/compose/fail2ban/jail.local:/etc/fail2ban/jail.d/cairn.local:ro" \
  "$UBUNTU" bash -c '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends fail2ban iptables >/dev/null 2>&1 || exit 3
    iptables -N DOCKER-USER
    log=/var/lib/docker/containers/abc/abc-json.log
    mkdir -p "${log%/*}" && : > "$log"
    fail2ban-server -b >/dev/null 2>&1
    for _ in $(seq 20); do fail2ban-client ping >/dev/null 2>&1 && break; sleep 0.5; done
    sleep 2
    for i in $(seq 8); do
      now=$(date -u +%Y-%m-%dT%H:%M:%S.%6NZ)
      printf "{\"log\":\"{\\\\\"timestamp\\\\\":\\\\\"%s\\\\\",\\\\\"level\\\\\":\\\\\"WARN\\\\\",\\\\\"event\\\\\":\\\\\"auth.login_failed\\\\\",\\\\\"ip\\\\\":\\\\\"203.0.113.7\\\\\",\\\\\"email\\\\\":\\\\\"admin@example.com\\\\\",\\\\\"reason\\\\\":\\\\\"bad_password\\\\\"}\\\\n\",\"stream\":\"stdout\",\"time\":\"%s\"}\n" "$now" "$now" >> "$log"
    done
    for _ in $(seq 30); do fail2ban-client status cairn 2>/dev/null | grep -q 203.0.113.7 && break; sleep 1; done
    fail2ban-client status cairn 2>&1
    iptables -S 2>&1
  ' 2>&1)
if grep -q 'Banned IP list:.*203\.0\.113\.7' <<< "$out" && grep -q -- '-A DOCKER-USER .*-j f2b-cairn' <<< "$out" \
  && grep -q -- '-A f2b-cairn -s 203.0.113.7/32 .*-j REJECT' <<< "$out"; then
  echo "fail2ban jail: ok"
else
  echo "FAIL fail2ban jail did not ban 203.0.113.7 in DOCKER-USER:"
  tail -25 <<< "$out" | sed 's/^/    /'
  exit 1
fi
