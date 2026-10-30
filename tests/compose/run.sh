#!/usr/bin/env bash
# Brings up each Compose example of the install guide against an image and checks what the guide promises.
# Usage: tests/compose/run.sh <image> [example...]   (examples: quick traefik tinyauth oidc; default all)
# Needs Docker with Compose v2.24+, curl, jq, openssl, python3 and script. Uses 127.0.0.1:18080 and :18443.
set -uo pipefail

IMAGE=${1:?usage: tests/compose/run.sh <image> [example...]}
shift
examples=("$@")
(( ${#examples[@]} )) || examples=(quick traefik tinyauth oidc)

source "$(dirname "$0")/lib.sh"
export CAIRN_TEST_IMAGE=$IMAGE
WORK=$(mktemp -d)
trap teardown EXIT

if [[ -n $(docker ps -aq --filter name='^cairn$') ]]; then
  say "A container named cairn exists already; the examples need that name. Remove it first."
  exit 1
fi

# ---- quick: plain HTTP on a home network ----

QUICK=http://127.0.0.1:18080

healthy() { [[ $(docker inspect -f '{{.State.Health.Status}}' cairn 2>/dev/null) == healthy ]]; }

admin_created() { admin | tee /dev/stderr | grep -q 'Created admin@example.com (admin, scope /).'; }

sign_in_quick() {
  [[ $(login "$QUICK" admin@example.com "$ADMIN_PASSWORD" "$WORK/quick.jar") == 200 ]] \
    && grep -q $'\tcairn_session\t' "$WORK/quick.jar"
}

me_is_admin() { # base jar [curl options...]
  local base=$1 jar=$2
  shift 2
  curl -s -b "$jar" "$@" "$base/api/me" | jq -e '.user.email == "admin@example.com"'
}

upload_lands_on_host() { # name
  local dir=$WORK/$1 file='Report #1 (50%) ✓ café.txt'
  printf 'Trail notes, day 1.\n' > "$WORK/upload.txt"
  local status
  status=$(curl -s -o /dev/null -w '%{http_code}' -b "$WORK/$1.jar" -H 'X-Cairn: 1' -X PUT \
    --data-binary @"$WORK/upload.txt" "$QUICK/api/upload/$(jq -rn --arg f "$file" '$f | @uri')")
  [[ $status == 2* ]] || { echo "upload answered $status"; return 1; }
  cmp "$WORK/upload.txt" "$dir/files/$file" || return 1
  local owner
  owner=$(stat -c %u "$dir/files/$file")
  [[ $owner == "${2:-10001}" ]] || { echo "owned by $owner"; return 1; }
}

backup_and_restore() {
  (cd "$WORK/quick" && eval "${README_COMMANDS[2]}" && eval "${README_COMMANDS[3]}") || return 1
  [[ -s $WORK/quick/cairn-backup.db ]] || { echo "no backup file"; return 1; }
  cp "$WORK/quick/cairn-backup.db" "$WORK/cairn-backup.db"
  # A fresh install elsewhere: empty volume, then the README's restore steps.
  down quick
  prepare quick restored
  cp "$WORK/cairn-backup.db" "$WORK/restored/cairn-backup.db"
  up quick restored || return 1
  (cd "$WORK/restored" && eval "${README_COMMANDS[4]}" && eval "${README_COMMANDS[5]}" && eval "${README_COMMANDS[6]}") \
    || return 1
  wait_for 30 healthy || return 1
  [[ $(login "$QUICK" admin@example.com "$ADMIN_PASSWORD" "$WORK/restored.jar") == 200 ]]
}

recreate_keeps_data() {
  (cd "$WORK/restored" && compose_cmd up -d --force-recreate --wait --wait-timeout 60 >/dev/null 2>&1) || return 1
  me_is_admin "$QUICK" "$WORK/restored.jar"
}

user_override_owns_uploads() {
  down restored
  prepare quick asuser
  # The files and data folders belong to the person running the test, as on a desktop where Cairn serves $HOME.
  as_root "$WORK/asuser" "chown $(id -u):$(id -g) /w/files"
  mkdir -p "$WORK/asuser/data"
  cat > "$WORK/asuser/user.yml" <<EOF
services:
  cairn:
    user: "$(id -u):$(id -g)"
    volumes: !override
      - ./files:/srv
      - ./data:/data
EOF
  up quick asuser "$WORK/asuser/user.yml" || return 1
  admin >/dev/null || return 1
  [[ $(login "$QUICK" admin@example.com "$ADMIN_PASSWORD" "$WORK/asuser.jar") == 200 ]] || return 1
  upload_lands_on_host asuser "$(id -u)"
}

data_not_writable_message() {
  down asuser
  mkdir -p "$WORK/rootdata"
  as_root "$WORK/rootdata" 'chown 0:0 /w && chmod 755 /w'
  local out rc
  out=$(docker run --rm -v "$WORK/rootdata:/data" "$IMAGE" 2>&1)
  rc=$?
  [[ $rc != 0 ]] || { echo "exit 0"; return 1; }
  grep -q '/data is not writable by this user (uid 10001 in the image). Change the folder.s owner or set user: in Compose.' \
    <<< "$out" || { echo "$out"; return 1; }
}

run_quick() {
  prepare quick quick
  CURRENT=quick
  COMPOSE_FILES=("$WORK/quick/compose.yml" "$TESTS/override/quick.yml")
  check override_only_allowed override_only_allowed quick quick
  if ! up quick quick; then check healthy false; return; fi
  check healthy healthy
  check readme_admin_command admin_created
  check password_sign_in sign_in_quick
  check upload_lands_on_host upload_lands_on_host quick
  check backup_and_restore backup_and_restore
  check recreate_keeps_data recreate_keeps_data
  check user_override_owns_uploads user_override_owns_uploads
  CURRENT=""
  check data_not_writable_message data_not_writable_message
  down asuser
}

for example in "${examples[@]}"; do
  case "$example" in
    quick) run_quick ;;
    *) say "unknown example $example"; FAILED=$((FAILED + 1)) ;;
  esac
done

echo "compose: $CHECKS checks, $FAILED failed"
(( FAILED == 0 ))
