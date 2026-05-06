#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${CARGO_HOME:-"$HOME/.cargo"}/bin"
CONFIG_DIR="$HOME/.tycho"
CONFIG_PATH="$CONFIG_DIR/depool-elect-config.json"
NODE_KEYS_PATH="${1:-${DEPOOL_ELECT_NODE_KEYS:-"$CONFIG_DIR/node_keys.json"}}"
SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
SERVICE_PATH="$SYSTEMD_USER_DIR/depool-elect.service"

mkdir -p "$BIN_DIR" "$CONFIG_DIR" "$SYSTEMD_USER_DIR"

cd "$ROOT_DIR"
cargo build --release
install -m 0755 "$ROOT_DIR/target/release/depool-elect" "$BIN_DIR/depool-elect"

echo "installed binary: $BIN_DIR/depool-elect"

if [[ -f "$CONFIG_PATH" ]]; then
    echo "config already exists: $CONFIG_PATH"
    echo "leaving existing config and keys unchanged"
else
    if [[ ! -f "$NODE_KEYS_PATH" ]]; then
        echo "missing node keys: $NODE_KEYS_PATH" >&2
        echo "run tycho node init first, or pass the real node keys path:" >&2
        echo "  ./install.sh /path/to/node_keys.json" >&2
        exit 1
    fi

    "$BIN_DIR/depool-elect" init-new \
        --config "$CONFIG_PATH" \
        --node-keys "$NODE_KEYS_PATH"

    echo "created config: $CONFIG_PATH"
fi

cat > "$SERVICE_PATH" <<SERVICE
[Unit]
Description=DePool election helper
After=network-online.target

[Service]
Type=simple
ExecStart=$BIN_DIR/depool-elect loop --config $CONFIG_PATH
Restart=always
RestartSec=30

[Install]
WantedBy=default.target
SERVICE

echo "created systemd user service: $SERVICE_PATH"

if command -v systemctl >/dev/null 2>&1; then
    systemctl --user daemon-reload || true
fi

echo "start with: systemctl --user start depool-elect"
echo "enable with: systemctl --user enable depool-elect"
