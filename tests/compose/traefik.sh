# Checks for examples/compose/traefik: Cairn behind Traefik with HTTPS and reCAPTCHA. Sourced by run.sh.

FILES=https://files.example.com

https_only() {
  local loc
  loc=$(curl -s -o /dev/null -w '%{redirect_url}' -H 'Host: files.example.com' http://127.0.0.1:18081/)
  [[ $loc == https://files.example.com/* ]] || { echo "redirect to '$loc'"; return 1; }
}

served_over_https() { [[ $(tcurl -o /dev/null -w '%{http_code}' "$FILES/api/health") == 200 ]]; }

captcha_on() {
  local config csp
  config=$(tcurl "$FILES/api/config")
  jq -e '.captcha.provider == "recaptcha" and .captcha.siteKey == "6Lc-your-site-key"' <<< "$config" \
    || { echo "$config"; return 1; }
  csp=$(tcurl -o /dev/null -D - "$FILES/login" | tr -d '\r' | sed -n 's/^content-security-policy: //Ip')
  grep -q "script-src 'self' https://www.google.com https://www.gstatic.com;" <<< "$csp" \
    && grep -q "frame-src[^;]*https://www.google.com" <<< "$csp" || { echo "$csp"; return 1; }
}

# Without a token the check fails closed, which also proves the secret came from its _FILE.
captcha_refuses_without_token() {
  local status
  status=$(login "$FILES" admin@example.com "$ADMIN_PASSWORD" "$WORK/traefik.jar" \
    --cacert "$WORK/ca/ca.pem" --connect-to files.example.com:443:127.0.0.1:18443)
  [[ $status == 400 ]] || { echo "status $status"; return 1; }
}

no_forwarded_ignored() { ! docker logs cairn 2>&1 | grep '"event":"http.forwarded_ignored"' >/dev/null; }

forwarded_ignored_once() {
  [[ $(docker logs cairn 2>&1 | grep -c '"event":"http.forwarded_ignored"') == 1 ]]
}

# The override check itself: an override that drops the example's Docker socket mount must not pass.
override_check_catches_removed_mount() {
  local saved=("${COMPOSE_FILES[@]}")
  printf 'services:\n  traefik:\n    volumes: !override\n      - letsencrypt:/letsencrypt\n      - %s:/certs:ro\n' "$WORK/ca" > "$WORK/traefik/drop.yml"
  COMPOSE_FILES+=("$WORK/traefik/drop.yml")
  if override_only_allowed traefik traefik >/dev/null 2>&1; then
    COMPOSE_FILES=("${saved[@]}")
    echo "an override that removes /var/run/docker.sock passed"
    return 1
  fi
  COMPOSE_FILES=("${saved[@]}")
}

run_traefik() {
  ca || { check ca false; return; }
  export CAIRN_TEST_CA=$WORK/ca
  prepare traefik traefik
  CURRENT=traefik
  COMPOSE_FILES=("$WORK/traefik/compose.yml" "$TESTS/override/traefik.yml")
  check override_only_allowed override_only_allowed traefik traefik
  check traefik_flags_match traefik_flags_match traefik traefik
  check override_check_catches_removed_mount override_check_catches_removed_mount
  if ! up traefik traefik; then check healthy false; return; fi
  check healthy healthy
  check https_only https_only
  check served_over_https served_over_https
  check captcha_on captcha_on
  check readme_admin_command admin_created
  check captcha_refuses_without_token captcha_refuses_without_token
  check forwarded_trusted no_forwarded_ignored
  # The same with CAIRN_TRUSTED_PROXIES empty: Cairn says once that it ignores Traefik's X-Forwarded-For.
  down traefik
  prepare traefik untrusted
  printf 'services:\n  cairn:\n    environment:\n      CAIRN_TRUSTED_PROXIES: ""\n' > "$WORK/untrusted/untrusted.yml"
  if up traefik untrusted "$WORK/untrusted/untrusted.yml"; then
    tcurl -o /dev/null "$FILES/api/config"
    tcurl -o /dev/null "$FILES/api/me"
    check forwarded_ignored_without_trusted_proxies forwarded_ignored_once
  else
    check forwarded_ignored_without_trusted_proxies false
  fi
  down untrusted
}
