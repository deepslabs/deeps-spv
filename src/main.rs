#![feature(async_closure)]

mod btcrpc;
mod chains;
mod chainstate;
mod config;
mod daemon;
mod ethrpc;
mod header;
mod index;
mod server;
mod solrpc;
mod store;
mod types;
mod utils;

use crate::chains::sol::SolClient;
use chainstate::ChainsState;
use daemon::Daemon;
use helios::ethereum::{database::ConfigDB, EthereumClient};
use helios::opstack::OpStackClient;
use index::Indexer;
use lazy_static::lazy_static;
use nakamoto::{client::Handle, net::poll::Waker};
use serde_json::Value;
use std::{path::Path, sync::Arc};
use store::Store;
use tokio::sync::RwLock;
use tracing::subscriber::set_global_default;
use tracing_subscriber::{fmt, EnvFilter};
use utils::SimpleCache;

lazy_static! {
    pub static ref RUNTIME: tokio::runtime::Runtime = tokio::runtime::Runtime::new().unwrap();
    pub static ref SOL_RUNTIME: tokio::runtime::Runtime = tokio::runtime::Runtime::new().unwrap();
    pub static ref HELIOS_RUMTIME: tokio::runtime::Runtime =
        tokio::runtime::Runtime::new().unwrap();
    pub static ref RELOAD: Arc<RwLock<bool>> = std::sync::Arc::new(RwLock::new(false));
}

/// Initialize configuration and run the SPV node
fn main() {
    let config = config::Config::from_args();
    setup_logging();
    tracing::info!(target: "spv", "config {config:?}");

    sgx_registration(&config.configcli);

    let (helios_eth_client, helios_op_client) = initialize_helios_client(&config);
    let (btc_handle, doge_handle) = initialize_p2p_clients(&config, &initialize_seal_key());
    let (btc_daemon, doge_daemon) = initialize_daemons(&config);
    let sol_client = SolClient::initialize_sol_client(&config).unwrap();

    let chains_state = initialize_chains_state(
        &Arc::new(Store::open(Path::new(&config.configcli.store))),
        btc_daemon,
        doge_daemon,
        btc_handle,
        doge_handle,
        helios_eth_client,
        helios_op_client,
        sol_client,
        &Arc::new(SimpleCache::new(2000)),
    );

    start_server(&chains_state, &config);
}

/// Sets up logging with environment filters and tracing subscriber
fn setup_logging() {
    let filter = EnvFilter::from_default_env();
    let subscriber = fmt().with_env_filter(filter).finish();
    set_global_default(subscriber).expect("setting default subscriber failed");
    tracing_log::LogTracer::init().unwrap();
}

/// Handles SGX registration based on configuration
fn sgx_registration(config: &config::ConfigCli) {
    if config.sgx_enable {
        let config = config.clone();
        RUNTIME.block_on(async {
            let current_version = sxn_rsv::register_sgx_2_not_fetch(
                config.subclient_url,
                30,
                0,
                config.device_owner,
                config.watcher_device_id,
                4,
            )
            .await;
            tracing::info!(target: "spv", "current_version {current_version:?}");
        });
        std::thread::sleep(std::time::Duration::from_secs(3));
    } else {
        RUNTIME.block_on(async {
            sxn_rsv::register_sgx_test(false).await;
        });
        tracing::info!(target: "spv", "register_sgx_test");
    }
}

/// Initializes Ethereum light client if enabled in configuration
fn initialize_helios_client(
    config: &config::Config,
) -> (
    Option<Arc<EthereumClient<ConfigDB>>>,
    Option<Arc<OpStackClient>>,
) {
    let eth_enabled = config.service.eth;
    let op_enabled = config.service.op;

    let config = &config.configcli;

    let eth_client = if eth_enabled {
        Some(Arc::new(crate::chains::run_eth_light_client(
            config.clone(),
        )))
    } else {
        None
    };

    let op_client = if op_enabled {
        Some(Arc::new(crate::chains::run_op_light_client(config.clone())))
    } else {
        None
    };

    (eth_client, op_client)
}

/// Initializes seal key for encryption
fn initialize_seal_key() -> Vec<u8> {
    sxn_rsv::ONLINESK
        .read()
        .unwrap()
        .as_ref()
        .unwrap()
        .as_bytes()
        .to_vec()
}

/// Initializes P2P clients for Bitcoin and Dogecoin based on configuration
fn initialize_p2p_clients(
    config: &config::Config,
    sealkey: &[u8],
) -> (Option<Arc<Handle<Waker>>>, Option<Arc<Handle<Waker>>>) {
    let btc_enabled = config.service.btc;
    let doge_enabled = config.service.doge;

    let config = &config.configcli;
    let sealkey = Some(sealkey.to_vec());

    let btc_handle = if btc_enabled {
        Some(
            chains::run_p2p_btc_client(
                &config.network_type.to_string(),
                &config.store,
                sealkey.clone(),
            )
            .expect("Failed to initialize Bitcoin P2P client"),
        )
    } else {
        None
    };

    let doge_handle = if doge_enabled {
        Some(
            chains::run_p2p_doge_client(config, sealkey)
                .expect("Failed to initialize Dogecoin P2P client"),
        )
    } else {
        None
    };

    (btc_handle, doge_handle)
}

/// Initializes daemon connections for Bitcoin and Dogecoin
fn initialize_daemons(config: &config::Config) -> (Option<Arc<Daemon>>, Option<Arc<Daemon>>) {
    let btc_enabled = config.service.btc;
    let doge_enabled = config.service.doge;

    let config = &config.configcli;

    let btc_daemon = if btc_enabled {
        Some(Arc::new(
            Daemon::new(
                config.daemon_rpc_addr,
                config.cookie.clone().unwrap(),
                config.network_type,
                config.electrs_support,
            )
            .unwrap(),
        ))
    } else {
        None
    };

    let doge_daemon = if doge_enabled {
        Some(Arc::new(
            Daemon::new(
                config.doge_daemon_rpc_addr,
                config.doge_cookie.clone().unwrap(),
                config.doge_network_type,
                config.electrs_support,
            )
            .unwrap(),
        ))
    } else {
        None
    };

    (btc_daemon, doge_daemon)
}

/// Initializes the chains state with all required components
fn initialize_chains_state(
    store: &Arc<Store>,
    btc_daemon: Option<Arc<Daemon>>,
    doge_daemon: Option<Arc<Daemon>>,
    btc_handle: Option<Arc<Handle<Waker>>>,
    doge_handle: Option<Arc<Handle<Waker>>>,
    helios_eth_client: Option<Arc<EthereumClient<ConfigDB>>>,
    helios_op_client: Option<Arc<OpStackClient>>,
    sol_client: Option<Arc<SolClient>>,
    response_cache: &Arc<SimpleCache<Vec<u8>, Value>>,
) -> Arc<ChainsState> {
    Arc::new(ChainsState::new(
        Arc::clone(store),
        btc_daemon,
        doge_daemon,
        btc_handle,
        doge_handle,
        helios_eth_client,
        helios_op_client,
        sol_client,
        Arc::clone(response_cache),
    ))
}

/// Starts the server based on configuration
fn start_server(chains_state: &Arc<ChainsState>, config: &config::Config) {
    let configcli = &config.configcli;

    if configcli.electrs_support || configcli.btc_only_p2p {
        crate::server::run_server(Arc::clone(chains_state), config.clone());
    }
}

/// Starts the indexer and syncs headers
#[allow(dead_code)]
fn start_indexer(store: &Arc<Store>, config: &config::Config, btc_daemon: &Arc<Daemon>) {
    let config = &config.configcli;

    let indexer = Indexer::open(Arc::clone(store), config);

    let btc_daemon = btc_daemon.clone();

    std::thread::spawn(move || {
        let mut tip = btc_daemon.getbestblockhash().unwrap();
        indexer.sync_headers(&btc_daemon, &tip).unwrap();
        indexer.full_compaction();

        loop {
            let current_tip = match btc_daemon.getbestblockhash() {
                Ok(res) => res,
                Err(e) => {
                    tracing::info!(target: "spv", "loop getbestblockhash failed {e}");
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    continue;
                }
            };

            if current_tip != tip {
                match indexer.sync_headers(&btc_daemon, &current_tip) {
                    Ok(res) => res,
                    Err(e) => {
                        tracing::info!(target: "spv", "loop sync_headers failed {e}, retry in 30 secs");
                        std::thread::sleep(std::time::Duration::from_secs(30));
                        continue;
                    }
                };
                tip = current_tip;
                tracing::info!(target: "spv", "sync new tip {current_tip}");
            };
            std::thread::sleep(std::time::Duration::from_secs(12));
        }
    });
}
