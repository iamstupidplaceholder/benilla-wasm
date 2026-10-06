#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
if [[ ! -f .env ]]; then
  cp .env.example .env
  echo 'Created .env. Set WOW_CLIENT_DIR and addresses, then run ./start.sh again.' >&2
  exit 1
fi
docker compose config --quiet
gzip -dc image.tar.gz | docker load
docker compose up -d --force-recreate
docker compose ps
