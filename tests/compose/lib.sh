# Helpers for tests/compose/run.sh. Sourced, not run.
# Each example runs from a copy in $WORK under the project name cairn-test-<example>, and is taken down with its
# volumes when the run ends.

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
TESTS=$ROOT/tests/compose
ALPINE=alpine@sha256:2e0b81d60597f449baa56dc25d355367869d2dca4cd57e866f1e2b52af3eb55f
ADMIN_PASSWORD='Trail-ridge 2026!'
CHECKS=0
FAILED=0
CURRENT=""   # the example whose logs a failed check prints
PROJECTS=()  # "project|dir|files..." of every project brought up, for the final teardown

# The exact commands the README gives. tests/compose/check-readme.sh fails when one is missing from the README.
# Sentences the README must carry word for word, because a reader acts on them.
README_SENTENCES=(
  "Docker opens published ports in the firewall itself, so ufw or firewalld rules do not close them"
)

README_COMMANDS=(
  'docker exec -it cairn /cairn user add "Admin" admin@example.com --admin --password'
  'sudo chown 10001:10001 files'
  'docker exec cairn /cairn backup /data/backup.db'
  'docker cp cairn:/data/backup.db ./cairn-backup.db'
  'docker compose stop cairn'
  "docker run --rm --volumes-from cairn -v \"\$PWD/cairn-backup.db:/backup.db:ro\" alpine:3.22 sh -c 'rm -f /data/cairn.db-wal /data/cairn.db-shm && cp /backup.db /data/cairn.db && chown 10001:10001 /data/cairn.db'"
  'docker compose start cairn'
  'docker compose pull && docker compose up -d'
)

say() { echo "$*" >&2; }

check() { # name command...
  local name=$1
  shift
  CHECKS=$((CHECKS + 1))
  # Not in a subshell: a check may bring projects up, and the teardown must know them.
  if "$@" > "$WORK/check.out" 2>&1; then
    echo "ok $name"
  else
    FAILED=$((FAILED + 1))
    echo "FAIL $name: $(tail -5 "$WORK/check.out" | tr '\n' ' ')"
    if [[ -n $CURRENT ]]; then (cd "$WORK/$CURRENT" && compose_cmd logs --tail 20 2>&1 | sed 's/^/    /'); fi
  fi
}

# Root-owned or 10001-owned files in $WORK are changed through a throwaway container: the test never needs sudo.
as_root() { # dir command
  docker run --rm -v "$1:/w" "$ALPINE" sh -c "$2"
}

# The Compose files of the current project: the example, its override, and any extra file.
COMPOSE_FILES=()
compose_cmd() {
  local args=()
  for f in "${COMPOSE_FILES[@]}"; do args+=(-f "$f"); done
  docker compose -p "cairn-test-$CURRENT" "${args[@]}" "$@"
}

# Copies an example into $WORK/<name> (the folder name doubles as the project), with a files folder owned by 10001
# and the placeholder secrets.
prepare() { # example name
  local example=$1 name=$2 dir=$WORK/$2
  mkdir -p "$dir"
  cp -r "$ROOT/examples/compose/$example/." "$dir/"
  cp "$ROOT/examples/compose/"*.yml "$WORK/" 2>/dev/null || true
  mkdir -p "$dir/files" "$dir/secrets"
  as_root "$dir" 'chown 10001:10001 /w/files'
  printf '%s\n' "${CAPTCHA_SECRET:-6Lc-placeholder-secret}" > "$dir/secrets/captcha_secret"
}

up() { # example name [extra compose files...]
  local example=$1 name=$2
  shift 2
  CURRENT=$name
  COMPOSE_FILES=("$WORK/$name/compose.yml" "$TESTS/override/$example.yml" "$@")
  # So that a plain `docker compose ...` from the README, run in the example's folder, reaches this project.
  export COMPOSE_PROJECT_NAME=cairn-test-$name
  COMPOSE_FILE=$(IFS=:; echo "${COMPOSE_FILES[*]}")
  export COMPOSE_FILE
  PROJECTS+=("$name|${COMPOSE_FILES[*]}")
  (cd "$WORK/$name" && compose_cmd up -d --wait --wait-timeout 180 >/dev/null 2>&1) || {
    say "up $name failed:"
    (cd "$WORK/$name" && compose_cmd logs --tail 30 2>&1 | sed 's/^/    /' >&2)
    return 1
  }
}

down() { # name
  local name=$1 entry files
  for entry in "${PROJECTS[@]}"; do
    [[ ${entry%%|*} == "$name" ]] || continue
    read -r -a files <<< "${entry#*|}"
    local args=()
    for f in "${files[@]}"; do args+=(-f "$f"); done
    (cd "$WORK/$name" && docker compose -p "cairn-test-$name" "${args[@]}" down -v --remove-orphans >/dev/null 2>&1)
  done
}

teardown() {
  local entry
  for entry in "${PROJECTS[@]}"; do down "${entry%%|*}"; done
  [[ -d $WORK ]] && as_root "$WORK" 'rm -rf /w/* /w/.[!.]*' >/dev/null 2>&1
  rm -rf "$WORK"
}

# The README's first-admin command, as typed: `script` gives `docker exec -it` the terminal it needs, and the
# password goes in through that terminal.
admin() {
  printf '%s\n' "$ADMIN_PASSWORD" | script -qec "${README_COMMANDS[0]}" /dev/null
}

# Prints the status of a password sign-in and keeps the cookie in <jar>.
login() { # base email password jar [curl options...]
  local base=$1 email=$2 password=$3 jar=$4
  shift 4
  curl -s -o /dev/null -w '%{http_code}' -c "$jar" -b "$jar" "$@" \
    -H 'X-Cairn: 1' -H 'Content-Type: application/json' \
    --data "$(jq -cn --arg e "$email" --arg p "$password" '{email: $e, password: $p, keep: true}')" \
    "$base/api/auth/login"
}

# The keys whose values differ between the example alone and the example with its override, as dotted paths.
# A path is allowed when it starts with a line of tests/compose/override/<example>.allow.
override_only_allowed() { # example name
  local example=$1 name=$2 dir=$WORK/$2 bad
  (cd "$dir" && docker compose -p "cairn-test-$name" -f compose.yml config --format json) > "$WORK/$name.alone.json" || return 1
  (cd "$dir" && compose_cmd config --format json) > "$WORK/$name.test.json" || return 1
  bad=$(python3 -I - "$WORK/$name.alone.json" "$WORK/$name.test.json" "$TESTS/override/$example.allow" <<'EOF'
import json, sys
a, b = (json.load(open(p)) for p in sys.argv[1:3])
allow = [l.strip() for l in open(sys.argv[3]) if l.strip() and not l.startswith("#")]
for k in ("name",):
    a.pop(k, None); b.pop(k, None)
diffs = []
def walk(x, y, path):
    if isinstance(x, dict) and isinstance(y, dict):
        for k in set(x) | set(y):
            walk(x.get(k), y.get(k), path + [str(k)])
    elif x != y:
        diffs.append(".".join(path))
walk(a, b, [])
print("\n".join(d for d in sorted(diffs) if not any(d == p or d.startswith(p + ".") for p in allow)))
EOF
  )
  [[ -z $bad ]] || { echo "override changes keys it may not: $bad"; return 1; }
}

wait_for() { # seconds command...
  local seconds=$1
  shift
  for _ in $(seq "$seconds"); do
    "$@" >/dev/null 2>&1 && return 0
    sleep 1
  done
  "$@"
}

# A throwaway CA and one certificate for example.com and *.example.com, in $WORK/ca, made once per run. Traefik
# serves the certificate; curl and the containers that call each other over HTTPS trust the CA.
ca() {
  local d=$WORK/ca
  [[ -s $d/example.com.pem ]] && return 0
  mkdir -p "$d"
  openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj '/CN=Cairn test CA' \
    -keyout "$d/ca.key" -out "$d/ca.pem" 2>/dev/null || return 1
  openssl req -newkey rsa:2048 -nodes -subj '/CN=example.com' \
    -keyout "$d/example.com.key" -out "$d/example.com.csr" 2>/dev/null || return 1
  printf 'subjectAltName=DNS:example.com,DNS:*.example.com\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' \
    > "$d/ext.cnf"
  openssl x509 -req -in "$d/example.com.csr" -CA "$d/ca.pem" -CAkey "$d/ca.key" -CAcreateserial -days 2 \
    -extfile "$d/ext.cnf" -out "$d/example.com.pem" 2>/dev/null || return 1
  printf 'tls:\n  stores:\n    default:\n      defaultCertificate:\n        certFile: /certs/example.com.pem\n        keyFile: /certs/example.com.key\n' \
    > "$d/tls.yml"
  chmod 644 "$d"/*
}

# curl through Traefik on 127.0.0.1:18443, trusting the test CA.
HOSTS=(files.example.com auth.example.com id.example.com)
tcurl() {
  local resolve=()
  for h in "${HOSTS[@]}"; do resolve+=(--connect-to "$h:443:127.0.0.1:18443"); done
  curl -s --cacert "$WORK/ca/ca.pem" "${resolve[@]}" "$@"
}

# The example's Traefik flags with Let's Encrypt swapped for the test certificate: the resolver flags go, the
# entrypoint keeps TLS on without a resolver, and the file provider serves the certificate.
traefik_flags_match() { # example name
  python3 -I - "$WORK/$2.alone.json" "$WORK/$2.test.json" <<'PY'
import json, sys
a, b = (json.load(open(p))["services"]["traefik"]["command"] for p in sys.argv[1:3])
want = [f.replace(".http.tls.certresolver=le", ".http.tls=true") for f in a if "acme" not in f]
want += ["--providers.file.filename=/certs/tls.yml"]
if b != want:
    print("override flags:", b, "want:", want)
    sys.exit(1)
PY
}
