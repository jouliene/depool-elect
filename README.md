# depool-elect

`depool-elect` is a small Tycho testnet helper for running a validator through a DePool.
It deploys the DePool, keeps the validator assurance staked in the pooling round,
keeps DePool/proxy balances funded, ticktocks only when elections are open, and sends
`participateInElections` only when the DePool round is ready.

## Install

Use real validator node keys. By default the installer expects:

```bash
~/.tycho/node_keys.json
```

Install the binary, create config, and create the user systemd service:

```bash
cd ~/RUST_CODE/depool-elect
./install.sh
```

If your node keys are elsewhere:

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

## Fund

The installer prints a `validator_wallet` address and a suggested top-up amount.
Send funds to the validator wallet only, not to the DePool address.

Check balance and DePool state:

```bash
depool-elect status --config ~/.tycho/depool-elect-config.json
```

## Run

Start once manually:

```bash
depool-elect once --config ~/.tycho/depool-elect-config.json
```

Run as a user service:

```bash
systemctl --user start depool-elect
```

Watch logs:

```bash
journalctl --user -u depool-elect -f
```

Enable autostart:

```bash
systemctl --user enable depool-elect
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
