#!/bin/zsh
# Creates a throwaway repo with real Docker objects joined to it, so the
# Docker path can be exercised end to end: discovery, the join by explicit
# evidence, and removal (which is permanent — the daemon has no Trash).
#
#   scripts/docker-probe.sh up     [root]   # default root: ~/src
#   scripts/docker-probe.sh down   [root]   # remove the objects and the repo
#
# The image is `FROM scratch`, so nothing is pulled and nothing is cached
# from the network; the payload is a 24 MB file so the row is visible.
set -euo pipefail
action="${1:-up}"
root="${2:-$HOME/src}"
name="slop-docker-probe"
repo="$root/$name"

case "$action" in
up)
  mkdir -p "$repo"
  cd "$repo"
  [ -d .git ] || { git init -q; git config user.email probe@example.com; git config user.name probe; }
  # A compose file whose `name:` is the join evidence, inside the worktree.
  cat > compose.yaml <<YAML
name: $name
services:
  app:
    image: ${name}:latest
    build: .
    volumes:
      - probe-data:/data
volumes:
  probe-data:
YAML
  cat > Dockerfile <<'DOCKER'
FROM scratch
COPY payload.bin /payload.bin
DOCKER
  # Deterministic bytes, no network.
  dd if=/dev/urandom of=payload.bin bs=1048576 count=24 status=none
  printf 'payload.bin\n' > .gitignore
  git add -A && git commit -qm "docker probe" || true

  docker build -q \
    --label com.docker.compose.project="$name" \
    -t "${name}:latest" . > /dev/null
  docker volume create \
    --label com.docker.compose.project="$name" probe-data > /dev/null
  echo "up: $repo"
  echo "  image  ${name}:latest"
  echo "  volume probe-data"
  echo "Now: slop-livin report $root --view docker --project $name"
  ;;
down)
  docker volume rm -f probe-data > /dev/null 2>&1 || true
  docker image rm -f "${name}:latest" > /dev/null 2>&1 || true
  rm -rf "$repo"
  echo "down: removed the image, the volume and $repo"
  ;;
*)
  echo "usage: $0 up|down [root]" >&2
  exit 2
  ;;
esac
