#!/usr/bin/env bash
set -euo pipefail

# Tear down the local Gitea instance and wipe its data + the generated gtx
# config. Use when you want a clean slate.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

docker compose down -v
rm -rf "$SCRIPT_DIR/data"
rm -f "$SCRIPT_DIR/gtx-config.toml"

echo "Local Gitea torn down."
