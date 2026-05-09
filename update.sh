#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SERVICE_NAME="${DEPOOL_ELECT_SERVICE:-depool-elect}"

cd "$ROOT_DIR"

echo "updating repository..."
git pull --ff-only

echo "installing depool-elect..."
"$ROOT_DIR/install.sh" "$@"

echo "restarting systemd user service: $SERVICE_NAME"
systemctl --user restart "$SERVICE_NAME"

echo "update complete"
