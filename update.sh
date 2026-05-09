#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${CARGO_HOME:-"$HOME/.cargo"}/bin"
SERVICE_NAME="${DEPOOL_ELECT_SERVICE:-depool-elect}"

cd "$ROOT_DIR"

echo "updating repository..."
git pull --ff-only

echo "building depool-elect..."
cargo build --release

mkdir -p "$BIN_DIR"
install -m 0755 "$ROOT_DIR/target/release/depool-elect" "$BIN_DIR/depool-elect"
echo "installed binary: $BIN_DIR/depool-elect"

if ! systemctl --user cat "$SERVICE_NAME" >/dev/null 2>&1; then
    echo "missing systemd user service: $SERVICE_NAME" >&2
    echo "run ./install.sh once before using ./update.sh" >&2
    exit 1
fi

echo "restarting systemd user service: $SERVICE_NAME"
systemctl --user restart "$SERVICE_NAME"

echo "update complete"
