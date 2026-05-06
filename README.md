# depool-elect

`depool-elect` is a small Tycho testnet helper for running a validator through a DePool.
It deploys the DePool, keeps the validator assurance staked in the pooling round,
keeps DePool/proxy balances funded, ticktocks only when elections are open, and sends
`participateInElections` only when the DePool round is ready.

## Install From Scratch

Open a terminal on the validator server.

Install basic tools if they are missing:

```bash
sudo apt update
sudo apt install -y git curl build-essential pkg-config libssl-dev
```

Install Rust if `cargo --version` does not work:

```bash
curl https://sh.rustup.rs -sSf | sh -s -- -y
. "$HOME/.cargo/env"
```

Clone the app:

```bash
cd ~
git clone https://github.com/jouliene/depool-elect.git
cd depool-elect
```

Check that your real Tycho validator node keys exist:

```bash
ls -l ~/.tycho/node_keys.json
```

Run the installer:

```bash
./install.sh
```

If your real node keys are not at `~/.tycho/node_keys.json`, pass the path:

```bash
./install.sh /path/to/node_keys.json
```

The installer creates:

```text
~/.cargo/bin/depool-elect
~/.tycho/depool-elect-config.json
~/.config/systemd/user/depool-elect.service
```

All DePool app state is stored in `~/.tycho/depool-elect-config.json`,
including copied node keys, the validator wallet keys, and the DePool keys.

## Fund The Validator Wallet

The installer prints a `validator_wallet` address and a suggested top-up amount.
Send funds to the validator wallet only, not to the DePool address.

Example output:

```text
INFO: validator_wallet=0:abc...
INFO: depool_address=0:def...
INFO: TOPUP validator_wallet=0:abc... with about 20182 TYCHO before `depool-elect once` (covers two validator-assurance rounds)
```

Send the suggested amount to `validator_wallet`.

Check balance and DePool state:

```bash
depool-elect status --config ~/.tycho/depool-elect-config.json
```

If the balance still shows `0`, wait a minute and run the same status command again.

## Start The App

Run one manual pass first:

```bash
depool-elect once --config ~/.tycho/depool-elect-config.json
```

Start the background user service:

```bash
systemctl --user start depool-elect
```

Watch logs:

```bash
journalctl --user -u depool-elect -f
```

Enable autostart after reboot:

```bash
systemctl --user enable depool-elect
```

Check service status:

```bash
systemctl --user status depool-elect
```

Stop the service:

```bash
systemctl --user stop depool-elect
```

## Update Later

Pull new code, reinstall the binary, and restart the service:

```bash
cd ~/depool-elect
git pull
./install.sh
systemctl --user restart depool-elect
```

## Notes

- If the DePool is deployed or first staked after elections are already open, the
  current election is skipped by DePool contract rules. The app will prepare for
  the next election cycle.
- Keep enough balance on the validator wallet for DePool deploy, assurance stake,
  proxy top-ups, ticktocks, and participation messages.
- Default DePool parameters are `min_stake=100` and `validator_assurance=10000`.
- Re-running `./install.sh` does not overwrite an existing config. If it finds an
  older split-file config, it rewrites it into the single config file format.
