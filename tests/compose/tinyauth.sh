# Checks for examples/compose/tinyauth: sign-in through Tinyauth behind Traefik, with Keycloak as Tinyauth's
# provider. Sourced by run.sh.

provider_listed() { # name
  tcurl "$FILES/api/config" | jq -e --arg n "$1" 'any(.providers[]; .name == $n)'
}

# Follows a sign-in button through the provider's login form as <email>; prints the last URL.
sso_flow() { # start-path email jar
  local start=$1 email=$2 jar=$3 page action last
  page=$(tcurl -L -c "$jar" -b "$jar" "$FILES$start")
  action=$(grep -o 'id="kc-form-login"[^>]*action="[^"]*"' <<< "$page" | sed 's/.*action="//; s/"$//; s/&amp;/\&/g')
  [[ -n $action ]] || { echo "no Keycloak login form" >&2; return 1; }
  last=$(tcurl -L -c "$jar" -b "$jar" -o /dev/null -w '%{url_effective}' \
    --data-urlencode "username=$email" --data-urlencode "password=$ADMIN_PASSWORD" "$action")
  # Tinyauth's /continue page sends the browser on to redirect_uri with JavaScript; curl does that step itself.
  if [[ $last == https://auth.example.com/continue\?* ]]; then
    local next
    next=$(python3 -I -c 'import sys, urllib.parse as u; print(u.parse_qs(u.urlsplit(sys.argv[1]).query)["redirect_uri"][0])' "$last")
    last=$(tcurl -L -c "$jar" -b "$jar" -o /dev/null -w '%{url_effective}' "$next")
  fi
  echo "$last"
}

broker_sign_in() {
  local jar=$WORK/broker.jar last
  last=$(sso_flow /auth/oauth/keycloak admin@example.com "$jar") || return 1
  [[ $last == "$FILES/" ]] || { echo "ended at $last"; return 1; }
  me_is_admin "$FILES" "$jar" --cacert "$WORK/ca/ca.pem" --connect-to files.example.com:443:127.0.0.1:18443 || return 1
  grep -q $'\t__Host-cairn_session\t' "$jar" || { echo "no __Host-cairn_session cookie"; return 1; }
  grep $'\t__Host-cairn_session\t' "$jar" | grep -q $'^#HttpOnly_files.example.com\tFALSE\t/\tTRUE\t' \
    || { echo "session cookie not Secure"; return 1; }
}

no_account() { # start-path
  local jar=$WORK/stranger.jar last
  rm -f "$jar"
  last=$(sso_flow "$1" stranger@example.com "$jar") || return 1
  [[ $last == "$FILES/login?error=no-account" ]] || { echo "ended at $last"; return 1; }
  tcurl -b "$jar" "$FILES/api/me" | jq -e '.user == null'
}

# Tinyauth stops the request before Cairn: a 401 for a client that is not a browser, a redirect to its login page
# for one that is. Cairn never sees it, so it logs nothing about it.
forged_remote_email_refused() {
  local jar=$WORK/forged.jar out before after
  rm -f "$jar"
  before=$(docker logs cairn 2>&1 | grep -c '"event":"auth\.')
  out=$(tcurl -c "$jar" -b "$jar" -o /dev/null -w '%{http_code} %{redirect_url}' \
    -H 'Remote-Email: admin@example.com' -H 'X-Auth-Secret: guess' "$FILES/auth/sso")
  [[ $out == "401 " || $out == 30?" https://auth.example.com/"* ]] || { echo "answered $out"; return 1; }
  tcurl -b "$jar" "$FILES/api/me" | jq -e '.user == null' || return 1
  after=$(docker logs cairn 2>&1 | grep -c '"event":"auth\.')
  [[ $before == "$after" ]] || { echo "Cairn logged an auth event for the forged request"; return 1; }
}

sso_path_variant_not_behind_broker() {
  local a b
  a=$(tcurl -o /dev/null -w '%{http_code}' "$FILES/auth/sso/")
  b=$(tcurl -o /dev/null -w '%{http_code}' "$FILES/AUTH/SSO")
  [[ $a == 404 && $b == 404 ]] || { echo "/auth/sso/ $a, /AUTH/SSO $b"; return 1; }
}

broker_sign_out() {
  tcurl -b "$WORK/broker.jar" -X POST -H 'X-Cairn: 1' "$FILES/api/auth/logout" \
    | jq -e '.next == "https://auth.example.com/logout"'
}

run_tinyauth() {
  ca || { check ca false; return; }
  export CAIRN_TEST_CA=$WORK/ca CAIRN_TEST_REALM=$TESTS/keycloak/realm.json
  prepare tinyauth tinyauth
  ADDONS=(keycloak.yml)
  CURRENT=tinyauth
  COMPOSE_FILES=("$WORK/tinyauth/compose.yml" "$WORK/keycloak.yml" "$TESTS/override/tinyauth.yml")
  check override_only_allowed override_only_allowed tinyauth tinyauth
  check traefik_flags_match traefik_flags_match tinyauth tinyauth
  if ! up tinyauth tinyauth; then check healthy false; ADDONS=(); return; fi
  check healthy healthy
  # The broker may have been unreachable at the first fetch; its buttons come back 20 s later.
  check broker_button wait_for 25 provider_listed Keycloak
  check readme_admin_command admin_created
  check broker_sign_in broker_sign_in
  check no_account no_account /auth/oauth/keycloak
  check forged_remote_email_refused forged_remote_email_refused
  check sso_path_variant_not_behind_broker sso_path_variant_not_behind_broker
  check broker_sign_out broker_sign_out
  down tinyauth
  ADDONS=()
}
