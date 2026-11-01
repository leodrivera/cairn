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
# The guide must quote the files this run checks.
"$TESTS/check-readme.sh" || exit 1
# The Fail2ban filter against forged lines, and the jail banning for real in a throwaway network namespace.
"$TESTS/check-fail2ban.sh" || exit 1
"$TESTS/check-fail2ban-jail.sh" || exit 1
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

# The same unwritable /data through Compose, as the README runs it: with restart: unless-stopped the container
# keeps restarting, and its log says why (the README tells people to look there).
data_not_writable_in_compose() {
  prepare quick rootdata2
  mkdir -p "$WORK/rootdata2/data"
  as_root "$WORK/rootdata2/data" 'chown 0:0 /w && chmod 755 /w'
  printf 'services:\n  cairn:\n    volumes: !override\n      - ./files:/srv\n      - ./data:/data\n' > "$WORK/rootdata2/data.yml"
  CURRENT=rootdata2
  COMPOSE_FILES=("$WORK/rootdata2/compose.yml" "$TESTS/override/quick.yml" "$WORK/rootdata2/data.yml")
  PROJECTS+=("rootdata2|${COMPOSE_FILES[*]}")
  (cd "$WORK/rootdata2" && compose_cmd up -d >/dev/null 2>&1) || return 1
  restarted() { (( $(docker inspect -f '{{.RestartCount}}' cairn) >= 1 )); }
  wait_for 30 restarted || { echo "no restart"; return 1; }
  docker logs cairn 2>&1 | grep -q '/data is not writable by this user (uid 10001 in the image)' \
    || { docker logs cairn 2>&1 | tail -3; return 1; }
}

# The README's upgrade command, for real: a newer image under the same tag in a throwaway registry, then
# `docker compose pull && docker compose up -d` must run it and keep the admin's session.
REGISTRY=cairn-test-registry
upgrade_pulls_new_image() {
  local repo=127.0.0.1:15001/cairn
  docker run -d --name "$REGISTRY" -p 127.0.0.1:15001:5000 \
    registry@sha256:a3d8aaa63ed8681a604f1dea0aa03f100d5895b6a58ace528858a7b332415373 >/dev/null || return 1
  docker tag "$IMAGE" "$repo:0.1" && docker push -q "$repo:0.1" >/dev/null || return 1
  prepare quick upgrade
  printf 'services:\n  cairn:\n    image: %s:0.1\n' "$repo" > "$WORK/upgrade/registry.yml"
  up quick upgrade "$WORK/upgrade/registry.yml" || return 1
  admin >/dev/null || return 1
  [[ $(login "$QUICK" admin@example.com "$ADMIN_PASSWORD" "$WORK/upgrade.jar") == 200 ]] || return 1
  printf 'FROM %s\nLABEL org.example.upgrade=2\n' "$IMAGE" | docker build -q -t "$repo:0.1" - >/dev/null || return 1
  docker push -q "$repo:0.1" >/dev/null || return 1
  (cd "$WORK/upgrade" && eval "${README_COMMANDS[7]}") >/dev/null 2>&1 || return 1
  wait_for 60 healthy || return 1
  [[ $(docker inspect -f '{{index .Config.Labels "org.example.upgrade"}}' cairn) == 2 ]] || { echo "old image still running"; return 1; }
  me_is_admin "$QUICK" "$WORK/upgrade.jar"
}

# One trash per mount, as on a NAS: ./files at /srv and ./photos at /srv/photos are two bind mounts, and rename does
# not cross them. A delete in /srv/photos must land in photos/.cairn-trash and come back on restore.
trash_per_mount() {
  prepare quick permount
  mkdir -p "$WORK/permount/photos" "$WORK/permount/camera"
  as_root "$WORK/permount" 'chown 10001:10001 /w/photos /w/camera && echo cat > /w/camera/cat.jpg && chown 10001:10001 /w/camera/cat.jpg'
  printf 'services:\n  cairn:\n    volumes: !override\n      - ./files:/srv\n      - ./photos:/srv/photos\n      - ./camera:/srv/media/camera\n      - cairn-data:/data\n' \
    > "$WORK/permount/mounts.yml"
  up quick permount "$WORK/permount/mounts.yml" || return 1
  admin >/dev/null || return 1
  local jar=$WORK/permount.jar
  [[ $(login "$QUICK" admin@example.com "$ADMIN_PASSWORD" "$jar") == 200 ]] || return 1
  local up_status
  up_status=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -H 'X-Cairn: 1' -X PUT --data-binary 'trail' \
    "$QUICK/api/upload/photos/a.txt")
  [[ $up_status == 201 ]] || { echo "upload answered $up_status"; return 1; }
  local res id
  res=$(curl -s -b "$jar" -H 'X-Cairn: 1' -H 'Content-Type: application/json' -X DELETE \
    --data '{"paths":["photos/a.txt"]}' "$QUICK/api/files")
  id=$(jq -r '.entries[0] // empty' <<< "$res")
  [[ -n $id ]] || { echo "delete answered $res"; return 1; }
  as_root "$WORK/permount" "test -f /w/photos/.cairn-trash/$id/a.txt" || { echo "not in photos/.cairn-trash"; return 1; }
  as_root "$WORK/permount" "test ! -e /w/files/.cairn-trash/$id" || { echo "crossed into the root's trash"; return 1; }
  res=$(curl -s -b "$jar" -H 'X-Cairn: 1' -H 'Content-Type: application/json' \
    --data "$(jq -cn --arg id "$id" '{ids: [$id]}')" "$QUICK/api/trash/restore")
  jq -e '.restored[0].path == "photos/a.txt"' <<< "$res" >/dev/null || { echo "restore answered $res"; return 1; }
  as_root "$WORK/permount" 'test "$(cat /w/photos/a.txt)" = trail' || { echo "not back in place"; return 1; }
}

# A folder holding a real mount point (./camera at /srv/media/camera) is refused by the trash and by a permanent
# delete, so the other disk is never carried into the trash or emptied. Runs on trash_per_mount's containers.
trash_refuses_mount_inside() {
  local jar=$WORK/permount.jar res
  for permanent in false true; do
    res=$(curl -s -b "$jar" -H 'X-Cairn: 1' -H 'Content-Type: application/json' -X DELETE \
      --data "$(jq -cn --argjson p "$permanent" '{paths: ["media"], permanent: $p}')" "$QUICK/api/files")
    jq -e '.deleted == 0 and .failed[0].error == "holds_mount"' <<< "$res" >/dev/null \
      || { echo "delete (permanent $permanent) answered $res"; return 1; }
  done
  as_root "$WORK/permount" 'test "$(cat /w/camera/cat.jpg)" = cat' || { echo "the mounted disk lost its file"; return 1; }
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
  check data_not_writable_in_compose data_not_writable_in_compose
  down rootdata2
  check upgrade_pulls_new_image upgrade_pulls_new_image
  down upgrade
  check trash_per_mount trash_per_mount
  check trash_refuses_mount_inside trash_refuses_mount_inside
  down permount
  docker rm -f "$REGISTRY" >/dev/null 2>&1
  docker image rm 127.0.0.1:15001/cairn:0.1 >/dev/null 2>&1
}

# The other examples keep their checks in a file of their own.
for f in "$TESTS"/*.sh; do
  [[ $f == */lib.sh || $f == */run.sh || $f == */check-*.sh ]] || source "$f"
done

for example in "${examples[@]}"; do
  case "$example" in
    quick) run_quick ;;
    traefik) run_traefik ;;
    tinyauth) run_tinyauth ;;
    oidc) run_oidc ;;
    *) say "unknown example $example"; FAILED=$((FAILED + 1)) ;;
  esac
done

echo "compose: $CHECKS checks, $FAILED failed"
(( FAILED == 0 ))
