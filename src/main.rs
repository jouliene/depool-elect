use anyhow::{Context, Result, bail};
use minik2::{
    Config as ChainConfig, DePool, DePoolParticipant, DePoolRound, Elector, EverWallet, KeyPair,
    SendReceipt, Transport, build_elections_data_to_sign, build_participate_in_elections_payload,
    build_ticktock_payload,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, sleep};

const DEFAULT_ENDPOINT: &str = "https://rpc-testnet.tychoprotocol.com";
const ONE: u128 = 1_000_000_000;
const BASECHAIN: i8 = 0;
const DEPOOL_WORKCHAIN: i8 = 0;
const DEFAULT_CONFIG_PATH: &str = "~/.tycho/depool-elect-config.json";
const DEFAULT_MIN_STAKE: &str = "100";
const DEFAULT_VALIDATOR_ASSURANCE: &str = "10000";
const DEFAULT_PARTICIPANT_REWARD_FRACTION: u8 = 95;
const DEFAULT_PARTICIPATE_VALUE: &str = "1";
const DEFAULT_TICKTOCK_VALUE: &str = "1";
const DEFAULT_WALLET_RESERVE: &str = "10";
const DEFAULT_STAKE_FACTOR: u32 = 3 * 65_536;
const DEPOOL_MIN_BALANCE: u128 = 20 * ONE;
const DEPOOL_TARGET_BALANCE: u128 = 30 * ONE;
const PROXY_MIN_BALANCE: u128 = 3 * ONE;
const PROXY_TARGET_BALANCE: u128 = 5 * ONE;
const ADD_STAKE_GAS: u128 = 500_000_000;
const UPDATE_ATTEMPTS: usize = 4;
const TICKTOCK_INTERVAL_SECS: u64 = 60;
const CONFIRMATION_ATTEMPTS: usize = 20;
const CONFIRMATION_INTERVAL_SECS: u64 = 3;
const ROUND_STEP_POOLING: u8 = 1;
const ROUND_STEP_WAITING_VALIDATOR_REQUEST: u8 = 2;
const COMPLETION_REASON_FAKE_ROUND: u8 = 2;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse(env::args().collect())?;

    match cli.command {
        Command::InitNew {
            force,
            node_keys_path,
            min_stake,
            validator_assurance,
            target_stake,
            participant_reward_fraction,
        } => {
            init_new(
                &cli.config_path,
                force,
                node_keys_path,
                min_stake,
                validator_assurance,
                target_stake,
                participant_reward_fraction,
            )
            .await
        }
        Command::Status => status(&cli.config_path).await,
        Command::Once => run_once(&cli.config_path).await,
        Command::MigrateConfig => migrate_config(&cli.config_path),
        Command::Loop => loop {
            if let Err(e) = run_once(&cli.config_path).await {
                log(format!("ERROR: {e:#}"));
            }
            sleep(Duration::from_secs(60)).await;
        },
    }
}

#[derive(Debug)]
struct Cli {
    config_path: PathBuf,
    command: Command,
}

#[derive(Debug)]
enum Command {
    InitNew {
        force: bool,
        node_keys_path: Option<PathBuf>,
        min_stake: String,
        validator_assurance: String,
        target_stake: Option<String>,
        participant_reward_fraction: u8,
    },
    Status,
    Once,
    MigrateConfig,
    Loop,
}

impl Cli {
    fn parse(args: Vec<String>) -> Result<Self> {
        let mut config_path = expand_home(Path::new(DEFAULT_CONFIG_PATH));
        let mut command = None;
        let mut force = false;
        let mut min_stake = DEFAULT_MIN_STAKE.to_owned();
        let mut validator_assurance = DEFAULT_VALIDATOR_ASSURANCE.to_owned();
        let mut target_stake = None;
        let mut node_keys_path = None;
        let mut participant_reward_fraction = DEFAULT_PARTICIPANT_REWARD_FRACTION;

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--config" => {
                    i += 1;
                    config_path = arg_path(&args, i, "--config")?;
                }
                "--force" => force = true,
                "--min-stake" => {
                    i += 1;
                    min_stake = arg_value(&args, i, "--min-stake")?;
                }
                "--validator-assurance" => {
                    i += 1;
                    validator_assurance = arg_value(&args, i, "--validator-assurance")?;
                }
                "--target-stake" => {
                    i += 1;
                    target_stake = Some(arg_value(&args, i, "--target-stake")?);
                }
                "--node-keys" => {
                    i += 1;
                    node_keys_path = Some(arg_path(&args, i, "--node-keys")?);
                }
                "--participant-reward-fraction" => {
                    i += 1;
                    participant_reward_fraction =
                        arg_value(&args, i, "--participant-reward-fraction")?
                            .parse()
                            .context("--participant-reward-fraction must be an integer")?;
                }
                "init-new" => {
                    command = Some(Command::InitNew {
                        force,
                        node_keys_path: node_keys_path.clone(),
                        min_stake: min_stake.clone(),
                        validator_assurance: validator_assurance.clone(),
                        target_stake: target_stake.clone(),
                        participant_reward_fraction,
                    });
                }
                "status" => command = Some(Command::Status),
                "once" => command = Some(Command::Once),
                "migrate-config" => command = Some(Command::MigrateConfig),
                "loop" => command = Some(Command::Loop),
                "help" | "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown argument `{other}`; run `depool-elect help`"),
            }
            i += 1;
        }

        let command = match command.context("missing command; run `depool-elect help`")? {
            Command::InitNew { .. } => Command::InitNew {
                force,
                node_keys_path,
                min_stake,
                validator_assurance,
                target_stake,
                participant_reward_fraction,
            },
            command => command,
        };
        Ok(Self {
            config_path,
            command,
        })
    }
}

fn print_help() {
    println!(
        "Usage:
  depool-elect init-new [--force] [--config PATH] [--target-stake TOKENS] [--validator-assurance TOKENS]
                         [--node-keys PATH]
  depool-elect status [--config PATH]
  depool-elect once [--config PATH]
  depool-elect migrate-config [--config PATH]
  depool-elect loop [--config PATH]

Defaults:
  endpoint: {DEFAULT_ENDPOINT}
  config:   {DEFAULT_CONFIG_PATH}
  DePool min stake: {DEFAULT_MIN_STAKE}
  validator assurance: {DEFAULT_VALIDATOR_ASSURANCE}
"
    );
}

fn arg_value(args: &[String], index: usize, name: &str) -> Result<String> {
    args.get(index)
        .cloned()
        .with_context(|| format!("{name} requires a value"))
}

fn arg_path(args: &[String], index: usize, name: &str) -> Result<PathBuf> {
    Ok(expand_home(Path::new(&arg_value(args, index, name)?)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    endpoint: String,
    node_keys: KeyFile,
    validator_wallet: KeyFile,
    depool_keys: KeyFile,
    depool_address: String,
    min_stake: String,
    validator_assurance: String,
    target_stake: Option<String>,
    participant_reward_fraction: u8,
    stake_factor: u32,
    participate_value: String,
    ticktock_value: String,
    wallet_reserve: String,
    retry: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyAppConfig {
    endpoint: String,
    node_keys_path: PathBuf,
    validator_wallet_path: PathBuf,
    depool_keys_path: PathBuf,
    depool_address: String,
    min_stake: String,
    validator_assurance: String,
    target_stake: Option<String>,
    participant_reward_fraction: u8,
    stake_factor: u32,
    participate_value: String,
    ticktock_value: String,
    wallet_reserve: String,
    retry: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum AppConfigFile {
    Current(AppConfig),
    Legacy(LegacyAppConfig),
}

impl LegacyAppConfig {
    fn into_current(self) -> Result<AppConfig> {
        Ok(AppConfig {
            endpoint: self.endpoint,
            node_keys: KeyFile::load(&self.node_keys_path)?,
            validator_wallet: KeyFile::load(&self.validator_wallet_path)?,
            depool_keys: KeyFile::load(&self.depool_keys_path)?,
            depool_address: self.depool_address,
            min_stake: self.min_stake,
            validator_assurance: self.validator_assurance,
            target_stake: self.target_stake,
            participant_reward_fraction: self.participant_reward_fraction,
            stake_factor: self.stake_factor,
            participate_value: self.participate_value,
            ticktock_value: self.ticktock_value,
            wallet_reserve: self.wallet_reserve,
            retry: self.retry,
        })
    }
}

fn load_config(config_path: &Path) -> Result<AppConfig> {
    let data = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config {}", config_path.display()))?;
    let config: AppConfigFile = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse {}", config_path.display()))?;
    match config {
        AppConfigFile::Current(config) => Ok(config),
        AppConfigFile::Legacy(config) => config.into_current(),
    }
}

fn migrate_config(config_path: &Path) -> Result<()> {
    let config = load_config(config_path)?;
    write_json(config_path, &config)?;
    log(format!("migrated config={}", config_path.display()));
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeyFile {
    public: String,
    secret: String,
}

impl KeyFile {
    fn generate() -> Self {
        Self::from_keypair(&KeyPair::generate())
    }

    fn from_keypair(keys: &KeyPair) -> Self {
        Self {
            public: keys.public_key_hex(),
            secret: keys.secret_key_hex(),
        }
    }

    fn load(path: &Path) -> Result<Self> {
        let data = fs::read_to_string(path)
            .with_context(|| format!("failed to read key file {}", path.display()))?;
        serde_json::from_str(&data).with_context(|| format!("failed to parse {}", path.display()))
    }

    fn keypair(&self) -> Result<KeyPair> {
        let keys = KeyPair::from_secret_hex(&self.secret)?;
        if keys.public_key_hex() != self.public {
            bail!("key file public key does not match secret");
        }
        Ok(keys)
    }
}

async fn init_new(
    config_path: &Path,
    force: bool,
    node_keys_path: Option<PathBuf>,
    min_stake: String,
    validator_assurance: String,
    target_stake: Option<String>,
    participant_reward_fraction: u8,
) -> Result<()> {
    if config_path.exists() && !force {
        bail!(
            "config {} already exists; pass --force to overwrite",
            config_path.display()
        );
    }
    if participant_reward_fraction == 0 || participant_reward_fraction >= 100 {
        bail!("participant reward fraction must be in 1..99");
    }

    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let default_node_keys_path = base_dir.join("node_keys.json");
    let node_keys = match node_keys_path {
        Some(path) => KeyFile::load(&path)?,
        None if default_node_keys_path.exists() => KeyFile::load(&default_node_keys_path)?,
        None => KeyFile::generate(),
    };
    let validator_wallet = KeyFile::generate();
    let depool_keys = KeyFile::generate();

    let transport = Transport::jrpc(DEFAULT_ENDPOINT)?;
    let wallet =
        EverWallet::with_workchain(transport.clone(), validator_wallet.keypair()?, BASECHAIN)?;
    let depool_address = DePool::compute_address(DEPOOL_WORKCHAIN, &depool_keys.keypair()?)?;

    let config = AppConfig {
        endpoint: DEFAULT_ENDPOINT.to_owned(),
        node_keys: node_keys.clone(),
        validator_wallet: validator_wallet.clone(),
        depool_keys: depool_keys.clone(),
        depool_address: depool_address.to_string(),
        min_stake,
        validator_assurance,
        target_stake,
        participant_reward_fraction,
        stake_factor: DEFAULT_STAKE_FACTOR,
        participate_value: DEFAULT_PARTICIPATE_VALUE.to_owned(),
        ticktock_value: DEFAULT_TICKTOCK_VALUE.to_owned(),
        wallet_reserve: DEFAULT_WALLET_RESERVE.to_owned(),
        retry: 5,
    };

    write_json(config_path, &config)?;

    let chain_config = ChainConfig::fetch(&transport).await?;
    let network_min = chain_config
        .validator_stake_params()?
        .min_stake
        .into_inner();
    let target = target_depool_stake(&config, None, network_min)?;
    let suggested_topup = target
        .saturating_mul(2)
        .saturating_add(minik2::MIN_BALANCE_FOR_DEPLOY)
        .saturating_add(DEPOOL_TARGET_BALANCE)
        .saturating_add(parse_tokens_to_nano(&config.participate_value)?)
        .saturating_add(parse_tokens_to_nano(&config.ticktock_value)?)
        .saturating_add(parse_tokens_to_nano(&config.wallet_reserve)?)
        .saturating_add(5 * ONE);

    log(format!("created config={}", config_path.display()));
    log(format!("validator_wallet={}", wallet.address()));
    log(format!("depool_address={}", config.depool_address));
    log(format!("node_public={}", node_keys.public));
    log(format!("network_min_stake={}", format_tokens(network_min)));
    log(format!("target_depool_stake={}", format_tokens(target)));
    log(format!(
        "TOPUP validator_wallet={} with about {} TYCHO before `depool-elect once` (covers two validator-assurance rounds)",
        wallet.address(),
        format_tokens(suggested_topup)
    ));
    Ok(())
}

async fn status(config_path: &Path) -> Result<()> {
    let loaded = Loaded::load(config_path)?;
    let mut wallet = loaded.wallet()?;
    let mut depool = loaded.depool()?;
    let chain_config = ChainConfig::fetch(&loaded.transport).await?;
    let elector = Elector::from_config(&loaded.transport, &chain_config)?;
    let elector_data = elector.get_data().await?;

    wallet.update().await?;
    depool.update().await?;

    log(format!("endpoint={}", loaded.config.endpoint));
    log(format!(
        "validator_wallet={} balance={}",
        wallet.address(),
        wallet.balance()
    ));
    log(format!(
        "depool={} active={} own_balance={} account_balance={}",
        depool.address,
        depool.is_active(),
        depool.own_balance,
        depool.account_balance
    ));
    log(format!(
        "depool_validator_wallet={}",
        depool
            .validator_wallet
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_owned())
    ));
    log(format!(
        "depool_proxies={}",
        format_addresses(&depool.proxies)
    ));
    log(format!("node_public={}", loaded.node_keys.public_key_hex()));

    if let Some(current) = elector_data.current_election() {
        log(format!(
            "current_election elect_at={} elect_close={} members={}",
            current.elect_at,
            current.elect_close,
            current.members.len()
        ));
    } else {
        log("current_election=none");
    }

    for round in depool.get_rounds() {
        log(format!("round {}", format_round(round)));
    }

    if let Some(participant) = depool.get_participant_info(wallet.address())? {
        log(format!(
            "participant_total={}",
            participant.total_round_stake
        ));
        for round in &participant.rounds {
            log(format!(
                "participant_round id={} total={} ordinary={}",
                round.round_id, round.total, round.ordinary
            ));
        }
    } else {
        log("participant=none");
    }

    Ok(())
}

async fn run_once(config_path: &Path) -> Result<()> {
    let loaded = Loaded::load(config_path)?;
    let mut wallet = loaded.wallet()?;
    let mut depool = loaded.depool()?;
    let chain_config = ChainConfig::fetch(&loaded.transport).await?;
    let elector = Elector::from_config(&loaded.transport, &chain_config)?;
    let elector_data = elector.get_data().await?;
    let current_election = elector_data
        .current_election()
        .map(|election| election.elect_at);

    wallet.update().await?;
    depool.update().await?;
    log(format!(
        "wallet={} balance={}",
        wallet.address(),
        wallet.balance()
    ));
    log(format!(
        "depool={} active={} own_balance={} account_balance={}",
        depool.address,
        depool.is_active(),
        depool.own_balance,
        depool.account_balance
    ));
    match current_election {
        Some(election_id) => log(format!("current_election={election_id}")),
        None => log("current_election=none"),
    }

    if !ensure_depool_deployed(&loaded, &mut wallet, &mut depool).await? {
        return Ok(());
    }
    if !maintain_depool_balances(&loaded, &mut wallet, &mut depool).await? {
        return Ok(());
    }

    depool.update().await?;
    if let Some(validator_wallet) = &depool.validator_wallet
        && validator_wallet != wallet.address()
    {
        bail!(
            "DePool validator wallet mismatch: depool has {}, config has {}",
            validator_wallet,
            wallet.address()
        );
    }

    let network_min = chain_config
        .validator_stake_params()?
        .min_stake
        .into_inner();
    let required_stake = target_depool_stake(&loaded.config, Some(&depool), network_min)?;
    if !ensure_pooling_validator_assurance(&loaded, &mut wallet, &mut depool, required_stake)
        .await?
    {
        return Ok(());
    }

    let Some(current) = elector.get_data().await?.current_election().cloned() else {
        log("elections are not open; DePool is funded and pooling stake is ready");
        return Ok(());
    };

    let Some(ready) = advance_depool_for_election(
        &loaded,
        &mut wallet,
        &mut depool,
        current.elect_at,
        required_stake,
    )
    .await?
    else {
        return Ok(());
    };

    if ready.step != ROUND_STEP_WAITING_VALIDATOR_REQUEST {
        log(format!(
            "depool target round is not WaitingValidatorRequest: {}",
            format_ready_round(&ready)
        ));
        return Ok(());
    }

    participate(
        &loaded,
        &elector,
        &mut wallet,
        &mut depool,
        current.elect_at,
        ready.id,
    )
    .await?;

    ensure_pooling_validator_assurance(&loaded, &mut wallet, &mut depool, required_stake).await?;
    Ok(())
}

struct Loaded {
    config: AppConfig,
    transport: Transport,
    node_keys: KeyPair,
    wallet_keys: KeyPair,
    depool_keys: KeyPair,
}

impl Loaded {
    fn load(config_path: &Path) -> Result<Self> {
        let config = load_config(config_path)?;
        let transport = Transport::jrpc(&config.endpoint)?;
        let node_keys = config.node_keys.keypair()?;
        let wallet_keys = config.validator_wallet.keypair()?;
        let depool_keys = config.depool_keys.keypair()?;
        Ok(Self {
            config,
            transport,
            node_keys,
            wallet_keys,
            depool_keys,
        })
    }

    fn wallet(&self) -> Result<EverWallet> {
        EverWallet::with_workchain(
            self.transport.clone(),
            KeyPair::from_secret_hex(self.wallet_keys.secret_key_hex())?,
            BASECHAIN,
        )
    }

    fn depool(&self) -> Result<DePool> {
        DePool::new(self.transport.clone(), self.config.depool_address.as_str())
    }
}

async fn ensure_depool_deployed(
    loaded: &Loaded,
    wallet: &mut EverWallet,
    depool: &mut DePool,
) -> Result<bool> {
    depool.update().await?;
    if depool.is_active() {
        return Ok(true);
    }

    if depool.account_balance < minik2::MIN_BALANCE_FOR_DEPLOY {
        let topup = minik2::MIN_BALANCE_FOR_DEPLOY.saturating_sub(depool.account_balance);
        if !wallet_has(
            wallet,
            topup.saturating_add(wallet_reserve(&loaded.config)?),
        )
        .await?
        {
            print_topup(
                wallet,
                topup.saturating_add(wallet_reserve(&loaded.config)?),
                "depool deploy funding",
            );
            return Ok(false);
        }

        log(format!("funding depool for deploy value={topup}"));
        let receipt = wallet
            .send_transaction_safe_with_retry(
                &depool.address,
                topup,
                false,
                3,
                None,
                loaded.config.retry,
            )
            .await?;
        log_receipt("depool_deploy_topup", &receipt);
        depool.update().await?;
    }

    if depool.account_balance < minik2::MIN_BALANCE_FOR_DEPLOY {
        print_topup(
            wallet,
            minik2::MIN_BALANCE_FOR_DEPLOY.saturating_sub(depool.account_balance),
            "depool deploy balance",
        );
        return Ok(false);
    }

    log(format!(
        "deploying depool={} min_stake={} validator_assurance={}",
        depool.address, loaded.config.min_stake, loaded.config.validator_assurance
    ));
    let receipt = depool
        .deploy(
            &loaded.depool_keys,
            parse_tokens_to_nano(&loaded.config.min_stake)?,
            parse_tokens_to_nano(&loaded.config.validator_assurance)?,
            wallet.address(),
            loaded.config.participant_reward_fraction,
        )
        .await?;
    log_receipt("depool_deploy", &receipt);
    depool.update().await?;
    Ok(true)
}

async fn maintain_depool_balances(
    loaded: &Loaded,
    wallet: &mut EverWallet,
    depool: &mut DePool,
) -> Result<bool> {
    depool.update().await?;
    if depool.own_balance < DEPOOL_MIN_BALANCE as i128 {
        let topup: u128 = (DEPOOL_TARGET_BALANCE as i128 - depool.own_balance)
            .max(0)
            .try_into()
            .context("depool topup does not fit u128")?;
        if wallet_has(
            wallet,
            topup.saturating_add(wallet_reserve(&loaded.config)?),
        )
        .await?
        {
            log(format!("depool balance topup value={topup}"));
            let receipt = depool.receive_funds(wallet, topup).await?;
            log_receipt("depool_balance_topup", &receipt);
            depool.update().await?;
        } else {
            print_topup(
                wallet,
                topup.saturating_add(wallet_reserve(&loaded.config)?),
                "depool balance",
            );
            return Ok(false);
        }
    }

    for proxy in depool.proxies.clone() {
        let balance = account_balance(&loaded.transport, &proxy).await?;
        if balance >= PROXY_MIN_BALANCE {
            log(format!(
                "proxy_balance_ready proxy={proxy} balance={balance}"
            ));
            continue;
        }

        let topup = PROXY_TARGET_BALANCE.saturating_sub(balance);
        if wallet_has(
            wallet,
            topup.saturating_add(wallet_reserve(&loaded.config)?),
        )
        .await?
        {
            log(format!("proxy topup proxy={proxy} value={topup}"));
            let receipt = wallet
                .send_transaction_safe_with_retry(
                    &proxy,
                    topup,
                    false,
                    3,
                    None,
                    loaded.config.retry,
                )
                .await?;
            log_receipt("proxy_topup", &receipt);
        } else {
            print_topup(
                wallet,
                topup.saturating_add(wallet_reserve(&loaded.config)?),
                "proxy balance",
            );
            return Ok(false);
        }
    }

    Ok(true)
}

#[derive(Debug, Clone)]
struct ReadyRound {
    id: u64,
    step: u8,
    supposed_elected_at: u32,
    stake: u64,
    validator_stake: u64,
}

async fn ensure_pooling_validator_assurance(
    loaded: &Loaded,
    wallet: &mut EverWallet,
    depool: &mut DePool,
    required_stake: u128,
) -> Result<bool> {
    depool.update().await?;
    let Some(pooling_round) = depool
        .get_rounds()
        .iter()
        .find(|round| round.step == ROUND_STEP_POOLING)
        .cloned()
    else {
        log("DePool has no pooling round; waiting for round rotation");
        return Ok(false);
    };

    let participant = depool.get_participant_info(wallet.address())?;
    let current_stake = participant_round_stake(participant, pooling_round.id);
    if current_stake >= required_stake {
        log(format!(
            "pooling_validator_stake_ready round_id={} current={} required={}",
            pooling_round.id, current_stake, required_stake
        ));
        return Ok(true);
    }

    let missing = required_stake.saturating_sub(current_stake);
    let stake_to_add = missing.max(depool.min_stake as u128);
    let value = stake_to_add.saturating_add(ADD_STAKE_GAS);
    if !wallet_has(
        wallet,
        value.saturating_add(wallet_reserve(&loaded.config)?),
    )
    .await?
    {
        print_topup(
            wallet,
            value.saturating_add(wallet_reserve(&loaded.config)?),
            "validator assurance stake",
        );
        return Ok(false);
    }

    log(format!(
        "adding validator assurance to pooling round_id={} current={} add={} required={}",
        pooling_round.id, current_stake, stake_to_add, required_stake
    ));
    let receipt = depool.add_ordinary_stake(wallet, stake_to_add).await?;
    log_receipt("add_ordinary_stake", &receipt);
    wait_for_pooling_stake_at_least(depool, wallet.address(), required_stake).await?;
    Ok(true)
}

async fn advance_depool_for_election(
    loaded: &Loaded,
    wallet: &mut EverWallet,
    depool: &mut DePool,
    election_id: u32,
    required_stake: u128,
) -> Result<Option<ReadyRound>> {
    let mut sent_ticktock = false;

    for attempt in 1..=UPDATE_ATTEMPTS {
        depool.update().await?;
        let rounds = depool.get_rounds();
        if rounds.len() < 3 {
            bail!(
                "DePool rounds number mismatch: expected at least 3, got {}",
                rounds.len()
            );
        }

        let prev_round_id = rounds[0].id;
        let target_round = ReadyRound {
            id: rounds[1].id,
            step: rounds[1].step,
            supposed_elected_at: rounds[1].supposed_elected_at,
            stake: rounds[1].stake,
            validator_stake: rounds[1].validator_stake,
        };
        let target_completion_reason = rounds[1].completion_reason;
        let pooling_round_id = rounds[2].id;
        let prev_round_log = format_round(&rounds[0]);
        let target_round_log = format_round(&rounds[1]);
        let pooling_round_log = format_round(&rounds[2]);
        let participant = depool.get_participant_info(wallet.address())?;
        let pooling_stake = participant_round_stake(participant, pooling_round_id)
            + participant_round_stake(participant, prev_round_id);

        log(format!(
            "depool_update attempt={attempt} election_id={election_id} required_stake={required_stake} pooling_stake={pooling_stake}"
        ));
        log(format!("prev_round {prev_round_log}"));
        log(format!("target_round {target_round_log}"));
        log(format!("pooling_round {pooling_round_log}"));

        if target_round.supposed_elected_at == election_id {
            return Ok(Some(target_round));
        }

        if sent_ticktock && target_completion_reason == COMPLETION_REASON_FAKE_ROUND {
            log(format!(
                "depool target round is fake after ticktock; this usually means DePool was deployed after current elections opened and will rotate for the next election target_round={target_round_log}"
            ));
            return Ok(None);
        }

        if attempt == UPDATE_ATTEMPTS {
            log(format!(
                "target round did not reach current election after ticktock; target_round={}",
                target_round_log
            ));
            return Ok(None);
        }

        let ticktock_value = parse_tokens_to_nano(&loaded.config.ticktock_value)?;
        if !wallet_has(
            wallet,
            ticktock_value.saturating_add(wallet_reserve(&loaded.config)?),
        )
        .await?
        {
            print_topup(
                wallet,
                ticktock_value.saturating_add(wallet_reserve(&loaded.config)?),
                "ticktock",
            );
            return Ok(None);
        }
        log(format!(
            "sending ticktock value={ticktock_value} attempt={attempt}"
        ));
        let receipt = send_depool_ticktock(loaded, wallet, depool, ticktock_value).await?;
        log_receipt("ticktock", &receipt);
        sent_ticktock = true;
        sleep(Duration::from_secs(TICKTOCK_INTERVAL_SECS)).await;
    }

    bail!("unreachable DePool update loop")
}

async fn participate(
    loaded: &Loaded,
    elector: &Elector,
    wallet: &mut EverWallet,
    depool: &mut DePool,
    election_id: u32,
    round_id: u64,
) -> Result<()> {
    depool.update().await?;
    if depool.proxies.is_empty() {
        bail!("DePool has no proxies");
    }

    let proxy = depool.proxies[(round_id as usize) % depool.proxies.len()].clone();
    let validator_key = minik2::HashBytes(loaded.node_keys.public_key_bytes());
    let adnl_addr = validator_key;

    let current = elector
        .get_data()
        .await?
        .current_election()
        .cloned()
        .context("elector has no current election")?;
    if let Some(member) = current.member(&validator_key) {
        log(format!(
            "already registered validator_key={} source={} stake={}",
            loaded.node_keys.public_key_hex(),
            member.src_addr,
            member.msg_value
        ));
        return Ok(());
    }

    let data = build_elections_data_to_sign(
        election_id,
        loaded.config.stake_factor,
        &proxy.address,
        &adnl_addr,
    );
    let data = loaded.transport.get_signature_context().await?.apply(&data);
    let signature = loaded.node_keys.sign(&data).to_bytes();
    let participate_value = parse_tokens_to_nano(&loaded.config.participate_value)?;

    if !wallet_has(
        wallet,
        participate_value.saturating_add(wallet_reserve(&loaded.config)?),
    )
    .await?
    {
        print_topup(
            wallet,
            participate_value.saturating_add(wallet_reserve(&loaded.config)?),
            "participate",
        );
        bail!("wallet balance is too low for participate");
    }

    log(format!(
        "participating election_id={election_id} round_id={round_id} proxy={proxy} validator_key={} adnl={}",
        loaded.node_keys.public_key_hex(),
        adnl_addr
    ));
    let receipt = send_depool_participate(
        loaded,
        wallet,
        depool,
        participate_value,
        validator_key,
        election_id,
        loaded.config.stake_factor,
        adnl_addr,
        signature,
    )
    .await?;
    log_receipt("participateInElections", &receipt);

    for attempt in 1..=CONFIRMATION_ATTEMPTS {
        if attempt > 1 {
            sleep(Duration::from_secs(CONFIRMATION_INTERVAL_SECS)).await;
        }
        let data = elector.get_data().await?;
        let Some(current) = data.current_election() else {
            log(format!(
                "confirmation attempt={attempt} no_current_election"
            ));
            continue;
        };
        if let Some(member) = current.member(&validator_key) {
            log(format!(
                "confirmed election_id={} source={} stake={}",
                current.elect_at, member.src_addr, member.msg_value
            ));
            return Ok(());
        }
        log(format!("confirmation attempt={attempt} not_registered"));
    }

    bail!("validator key not registered after confirmation timeout")
}

async fn send_depool_ticktock(
    loaded: &Loaded,
    wallet: &mut EverWallet,
    depool: &DePool,
    value: u128,
) -> Result<SendReceipt> {
    let payload = build_ticktock_payload()?;
    wallet
        .send_transaction_safe_with_retry(
            &depool.address,
            value,
            true,
            3,
            Some(&payload),
            loaded.config.retry,
        )
        .await
}

#[allow(clippy::too_many_arguments)]
async fn send_depool_participate(
    loaded: &Loaded,
    wallet: &mut EverWallet,
    depool: &DePool,
    value: u128,
    validator_key: minik2::HashBytes,
    stake_at: u32,
    max_factor: u32,
    adnl_addr: minik2::HashBytes,
    signature: impl AsRef<[u8]>,
) -> Result<SendReceipt> {
    let payload = build_participate_in_elections_payload(
        now_millis()?,
        validator_key,
        stake_at,
        max_factor,
        adnl_addr,
        signature,
    )?;
    wallet
        .send_transaction_safe_with_retry(
            &depool.address,
            value,
            true,
            3,
            Some(&payload),
            loaded.config.retry,
        )
        .await
}

fn target_depool_stake(
    config: &AppConfig,
    depool: Option<&DePool>,
    network_min: u128,
) -> Result<u128> {
    let configured = match &config.target_stake {
        Some(value) => parse_tokens_to_nano(value)?,
        None => 0,
    };
    let depool_required = depool
        .map(|depool| depool.validator_assurance.max(depool.min_stake) as u128)
        .unwrap_or_else(|| {
            parse_tokens_to_nano(&config.validator_assurance)
                .unwrap_or(0)
                .max(parse_tokens_to_nano(&config.min_stake).unwrap_or(0))
        });
    Ok(configured.max(depool_required).max(network_min))
}

fn participant_round_stake(participant: Option<&DePoolParticipant>, round_id: u64) -> u128 {
    participant
        .and_then(|participant| {
            participant
                .rounds
                .iter()
                .find(|round| round.round_id == round_id)
        })
        .map(|round| round.total as u128)
        .unwrap_or_default()
}

fn current_pooling_stake(depool: &DePool, participant: &minik2::StdAddr) -> Result<Option<u128>> {
    let Some(pooling_round) = depool.get_rounds().iter().find(|round| round.step == 1) else {
        return Ok(None);
    };
    Ok(Some(participant_round_stake(
        depool.get_participant_info(participant)?,
        pooling_round.id,
    )))
}

async fn wait_for_pooling_stake_at_least(
    depool: &mut DePool,
    participant: &minik2::StdAddr,
    threshold: u128,
) -> Result<()> {
    for attempt in 1..=CONFIRMATION_ATTEMPTS {
        if attempt > 1 {
            sleep(Duration::from_secs(CONFIRMATION_INTERVAL_SECS)).await;
        }
        depool.update().await?;
        let stake = current_pooling_stake(depool, participant)?.unwrap_or_default();
        log(format!(
            "stake_confirm attempt={attempt} pooling_stake={stake}"
        ));
        if stake >= threshold {
            return Ok(());
        }
    }

    bail!("pooling stake did not reach {threshold}");
}

async fn wallet_has(wallet: &mut EverWallet, required: u128) -> Result<bool> {
    wallet.update().await?;
    Ok(wallet.balance() >= required)
}

fn print_topup(wallet: &EverWallet, required: u128, reason: &str) {
    log(format!(
        "TOPUP validator_wallet={} with_at_least={} reason={} current_balance={}",
        wallet.address(),
        required.saturating_sub(wallet.balance()),
        reason,
        wallet.balance()
    ));
}

fn wallet_reserve(config: &AppConfig) -> Result<u128> {
    parse_tokens_to_nano(&config.wallet_reserve)
}

async fn account_balance(transport: &Transport, address: &minik2::StdAddr) -> Result<u128> {
    Ok(transport
        .get_account_state(address.to_string())
        .await?
        .account()
        .map(|account| account.balance.tokens.into())
        .unwrap_or_default())
}

fn format_ready_round(round: &ReadyRound) -> String {
    format!(
        "{{id={}, supposed_elected_at={}, step={}, stake={}, validator_stake={}}}",
        round.id, round.supposed_elected_at, round.step, round.stake, round.validator_stake
    )
}

fn format_round(round: &DePoolRound) -> String {
    format!(
        "{{id={}, supposed_elected_at={}, step={}, stake={}, validator_stake={}, completion_reason={}}}",
        round.id,
        round.supposed_elected_at,
        round.step,
        round.stake,
        round.validator_stake,
        round.completion_reason
    )
}

fn format_addresses(addresses: &[minik2::StdAddr]) -> String {
    if addresses.is_empty() {
        return "none".to_owned();
    }
    addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_tokens_to_nano(value: &str) -> Result<u128> {
    let value = value.trim();
    if value.is_empty() {
        bail!("token amount is empty");
    }
    let (whole, frac) = value.split_once('.').unwrap_or((value, ""));
    let whole = whole
        .parse::<u128>()
        .with_context(|| format!("invalid token amount `{value}`"))?;
    if frac.len() > 9 {
        bail!("token amount has more than 9 decimal places");
    }
    let mut frac = frac.to_owned();
    while frac.len() < 9 {
        frac.push('0');
    }
    let frac = if frac.is_empty() {
        0
    } else {
        frac.parse::<u128>()
            .with_context(|| format!("invalid token amount `{value}`"))?
    };
    whole
        .checked_mul(ONE)
        .and_then(|value| value.checked_add(frac))
        .context("token amount is too large")
}

fn format_tokens(value: u128) -> String {
    let whole = value / ONE;
    let frac = value % ONE;
    if frac == 0 {
        return whole.to_string();
    }
    let mut frac = format!("{frac:09}");
    while frac.ends_with('0') {
        frac.pop();
    }
    format!("{whole}.{frac}")
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let data = serde_json::to_string_pretty(value)?;
    fs::write(path, data).with_context(|| format!("failed to write {}", path.display()))?;
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path)
            .with_context(|| format!("failed to stat {}", path.display()))?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to protect {}", path.display()))?;
    }
    Ok(())
}

fn expand_home(path: &Path) -> PathBuf {
    let Some(path) = path.to_str() else {
        return path.to_owned();
    };
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

fn now_millis() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX epoch")?
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX))
}

fn log(message: impl AsRef<str>) {
    println!("INFO: {}", message.as_ref());
}

fn log_receipt(label: &str, receipt: &SendReceipt) {
    log(format!("{label}_hash={}", receipt.message_hash));
}
