use alloy::primitives::B256;
use helios::ethereum::{
    config::checkpoints, config::networks::Network, database::ConfigDB, EthereumClient,
    EthereumClientBuilder,
};
use std::{
    str::FromStr,
    sync::mpsc::{self},
};

use crate::config::ConfigCli;
pub use crate::HELIOS_RUMTIME;

pub struct EthConfig {
    consensus_rpc: String,
    execution_rpc: String,
    network: String,
    subclient_url: String,
    sgx_enable: bool,
}

pub fn run_eth_light_client(config: ConfigCli) -> EthereumClient<ConfigDB> {
    tracing::info!(target: "spv", "start eth helios");

    let eth_config = EthConfig {
        consensus_rpc: config.consensus_rpc,
        execution_rpc: config.execution_rpc,
        network: config.eth_network,
        subclient_url: config.subclient_url,
        sgx_enable: config.sgx_enable,
    };

    let (tx2, rx2) = mpsc::channel::<EthereumClient<ConfigDB>>();

    HELIOS_RUMTIME.spawn(async move {
        let client = start_helios(eth_config).await.unwrap();
        let _ = tx2.send(client);
    });

    rx2.recv().unwrap()
}

pub async fn start_helios(eth_config: EthConfig) -> Result<EthereumClient<ConfigDB>, String> {
    let EthConfig {
        consensus_rpc,
        execution_rpc,
        network,
        subclient_url,
        sgx_enable,
    } = eth_config;

    let network = Network::from_str(&network).unwrap();

    let checkpoint = if sgx_enable {
        let eth_checkpoint = sxn_rsv::fetch_eth_checkpoint(subclient_url)
            .await
            .unwrap();
        B256::from_slice(&eth_checkpoint)
    } else {
        let cf = checkpoints::CheckpointFallback::new()
            .build()
            .await
            .unwrap();
        cf.fetch_latest_checkpoint(&network).await.unwrap()
    };

    println!("Fetched latest {network} checkpoint: {checkpoint} from bool = {sgx_enable}");

    let mut client: EthereumClient<ConfigDB> = EthereumClientBuilder::new()
        .network(network)
        .consensus_rpc(&consensus_rpc)
        .execution_rpc(&execution_rpc)
        .data_dir("./db/data".into())
        .checkpoint(checkpoint)
        .build()
        .map_err(|e| format!("client new {e}"))?;

    client
        .start()
        .await
        .map_err(|e| format!("client start {e}"))?;

    tracing::info!(target: "spv", "wait {network} eth synced");
    client.wait_synced().await;
    tracing::info!(target: "spv", "{network} eth synced");

    Ok(client)
}
