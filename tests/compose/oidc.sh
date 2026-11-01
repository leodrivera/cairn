# Checks for examples/compose/oidc with keycloak.yml and own-ca.yml: direct OIDC behind Traefik, a provider whose
# certificate comes from an own CA, trusted proxies and the Fail2ban filter. Sourced by run.sh.

# crazymax/fail2ban:1.1.0, for fail2ban-regex only.
FAIL2BAN=crazymax/fail2ban@sha256:2bc26b68f373ce6e52bbd181de2937a6694f72b74ddf15bbd1b25e90553dcda3

log_has() { docker logs cairn 2>&1 | grep "\"event\":\"$1\"" >/dev/null; }

no_provider() { tcurl "$FILES/api/config" | jq -e '.providers == []'; }

without_own_ca_unreachable() {
  wait_for 30 log_has oidc.unreachable || { echo "no oidc.unreachable"; return 1; }
  no_provider
}

oidc_sign_in() {
  local jar=$WORK/oidc.jar last
  last=$(sso_flow /auth/oidc/google admin@example.com "$jar") || return 1
  [[ $last == "$FILES/" ]] || { echo "ended at $last"; return 1; }
  me_is_admin "$FILES" "$jar" --cacert "$WORK/ca/ca.pem" --connect-to files.example.com:443:127.0.0.1:18443 || return 1
  grep -q $'\t__Host-cairn_session\t' "$jar" || { echo "no __Host-cairn_session cookie"; return 1; }
}

unverified_refused() {
  local jar=$WORK/unverified.jar last
  last=$(sso_flow /auth/oidc/google unverified@example.com "$jar") || return 1
  [[ $last == "$FILES/login?error=sso" ]] || { echo "ended at $last"; return 1; }
}

wrong_password() { # email
  [[ $(login "$FILES" "$1" 'Wrong-password 1!' "$WORK/wrong.jar" \
    --cacert "$WORK/ca/ca.pem" --connect-to files.example.com:443:127.0.0.1:18443) == 401 ]]
}

# The address every failed sign-in was logged with, one per line.
failed_ips() { docker logs cairn 2>&1 | grep '"event":"auth.login_failed"' | jq -r .ip; }

# Three wrong passwords, a typed e-mail that carries an address, a right sign-in, a 403 and a 404: the log names
# the host's address on the network (Traefik passed it on), never Traefik's own.
trusted_proxies_effect() {
  local gateway traefik_ip
  gateway=$(docker network inspect cairn-test-oidc_default -f '{{(index .IPAM.Config 0).Gateway}}')
  for _ in 1 2 3; do wrong_password admin@example.com || return 1; done
  wrong_password 'x","ip":"198.51.100.67' || return 1
  [[ $(login "$FILES" admin@example.com "$ADMIN_PASSWORD" "$WORK/right.jar" \
    --cacert "$WORK/ca/ca.pem" --connect-to files.example.com:443:127.0.0.1:18443) == 200 ]] || return 1
  [[ $(tcurl -o /dev/null -w '%{http_code}' -X POST "$FILES/api/auth/logout") == 403 ]] || return 1
  [[ $(tcurl -o /dev/null -w '%{http_code}' "$FILES/s/short") == 404 ]] || return 1
  [[ $(failed_ips | sort -u) == "$gateway" ]] || { echo "logged $(failed_ips | tr '\n' ' '), want $gateway"; return 1; }
  [[ $(failed_ips | wc -l) == 4 ]] || { echo "$(failed_ips | wc -l) failed sign-ins logged"; return 1; }
  traefik_ip=$(docker inspect cairn-test-oidc-traefik-1 -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}')
  [[ $gateway != "$traefik_ip" ]] || { echo "gateway equals Traefik's address"; return 1; }
}

# The jail's own input: Docker's json-file log of the container, read through the README's filter.
fail2ban_counts_only_failed_sign_ins() {
  local id gateway out
  id=$(docker inspect cairn -f '{{.Id}}')
  gateway=$(docker network inspect cairn-test-oidc_default -f '{{(index .IPAM.Config 0).Gateway}}')
  # -o ip prints the address Fail2ban would count, once per matched line.
  out=$(docker run --rm -v "/var/lib/docker/containers/$id:/logs:ro" \
    -v "$ROOT/examples/compose/fail2ban/cairn.conf:/filter.conf:ro" \
    --entrypoint fail2ban-regex "$FAIL2BAN" -o ip "/logs/$id-json.log" /filter.conf 2>&1)
  [[ $(grep -c . <<< "$out") == 4 && $(sort -u <<< "$out") == "$gateway" ]] \
    || { echo "counted: $(tr '\n' ' ' <<< "$out"), want 4 times $gateway"; return 1; }
}

share_link_uses_public_url() {
  local jar=$WORK/right.jar
  printf 'Trail notes.\n' > "$WORK/notes.txt"
  tcurl -b "$jar" -o /dev/null -H 'X-Cairn: 1' -X PUT --data-binary @"$WORK/notes.txt" "$FILES/api/upload/notes.txt" \
    || return 1
  tcurl -b "$jar" -H 'X-Cairn: 1' -H 'Content-Type: application/json' \
    --data '{"path":"notes.txt","expiresDays":7,"password":null,"download":true}' "$FILES/api/shares" \
    | jq -e '.share.url | startswith("https://files.example.com/s/")'
}

untrusted_ip_is_traefik() {
  wrong_password admin@example.com || return 1
  local traefik_ip
  traefik_ip=$(docker inspect cairn-test-oidc2-traefik-1 -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}')
  [[ $(failed_ips) == "$traefik_ip" ]] || { echo "logged $(failed_ips), Traefik is $traefik_ip"; return 1; }
  log_has http.forwarded_ignored
}

run_oidc() {
  ca || { check ca false; return; }
  export CAIRN_TEST_CA=$WORK/ca CAIRN_TEST_REALM=$TESTS/keycloak/realm.json

  # Without the own CA, Cairn cannot reach the provider and shows no button.
  prepare oidc noca
  ADDONS=(keycloak.yml)
  if up oidc noca; then
    check without_own_ca_unreachable without_own_ca_unreachable
  else
    check without_own_ca_unreachable false
  fi
  down noca

  prepare oidc oidc
  cp "$WORK/ca/ca.pem" "$WORK/oidc/ca.pem"
  ADDONS=(keycloak.yml own-ca.yml)
  CURRENT=oidc
  COMPOSE_FILES=("$WORK/oidc/compose.yml" "$WORK/keycloak.yml" "$WORK/own-ca.yml" "$TESTS/override/oidc.yml")
  check override_only_allowed override_only_allowed oidc oidc
  check traefik_flags_match traefik_flags_match oidc oidc
  if ! up oidc oidc; then check healthy false; ADDONS=(); return; fi
  check healthy healthy
  # Keycloak was still starting when Cairn first asked; Cairn asks again every minute.
  check own_ca_trusted wait_for 75 provider_listed Google
  check oidc_ready log_has oidc.ready
  check readme_admin_command admin_created
  check oidc_sign_in oidc_sign_in
  check no_account no_account /auth/oidc/google
  check unverified_refused unverified_refused
  check trusted_proxies_effect trusted_proxies_effect
  check fail2ban_counts_only_failed_sign_ins fail2ban_counts_only_failed_sign_ins
  check share_link_uses_public_url share_link_uses_public_url
  down oidc

  # The same with CAIRN_TRUSTED_PROXIES empty: Traefik's address is logged, which is what Fail2ban would ban.
  prepare oidc oidc2
  cp "$WORK/ca/ca.pem" "$WORK/oidc2/ca.pem"
  printf 'services:\n  cairn:\n    environment:\n      CAIRN_TRUSTED_PROXIES: ""\n' > "$WORK/oidc2/untrusted.yml"
  if up oidc oidc2 "$WORK/oidc2/untrusted.yml"; then
    check untrusted_ip_is_traefik untrusted_ip_is_traefik
  else
    check untrusted_ip_is_traefik false
  fi
  down oidc2
  ADDONS=()
}
