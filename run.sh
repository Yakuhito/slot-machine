#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

: "${LAUNCHER_IDS:=c98d2ea5fbaa6507c2d56598b81833c6c643f1ead0062c4641ac1f9d94331c4a}"

docker build -t ghcr.io/yakuhito/slot-machine .

mkdir -p xchandles-data
docker rm -f xchandles-api >/dev/null 2>&1 || true

docker run -d \
  --name xchandles-api \
  --restart unless-stopped \
  --user "$(id -u):$(id -g)" \
  -p 127.0.0.1:8080:8080 \
  -v "$(pwd)/xchandles-data:/data" \
  -e BIND_ADDR=0.0.0.0:8080 \
  ghcr.io/yakuhito/slot-machine \
  xchandles listen --testnet11 --launcher-ids "$LAUNCHER_IDS"
