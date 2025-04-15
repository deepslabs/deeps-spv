use helios::opstack::{config::Network, OpStackClient, OpStackClientBuilder};
use std::{
    str::FromStr,
    sync::mpsc::{self},
};

use crate::config::ConfigCli;
pub use crate::HELIOS_RUMTIME;

pub struct OPConfig {
    consensus_rpc: String,
    execution_rpc: String,
    network: String,
}

pub fn run_op_light_client(config: ConfigCli) -> OpStackClient {
    tracing::info!(target: "spv", "start optimism helios");

    let op_config = OPConfig {
        consensus_rpc: config.op_consensus_rpc,
        execution_rpc: config.op_execution_rpc,
        network: config.op_network,
    };

    let (tx2, rx2) = mpsc::channel::<OpStackClient>();

    HELIOS_RUMTIME.spawn(async move {
        let client = start_helios(op_config).await.unwrap();
        let _ = tx2.send(client);
    });
    rx2.recv().unwrap()
}

pub async fn start_helios(op_config: OPConfig) -> Result<OpStackClient, String> {
    let OPConfig {
        consensus_rpc,
        execution_rpc,
        network,
    } = op_config;

    let network = Network::from_str(&network).unwrap();

    let client: OpStackClient = OpStackClientBuilder::new()
        .network(network)
        .consensus_rpc(&consensus_rpc)
        .execution_rpc(&execution_rpc)
        .build()
        .map_err(|e| format!("client new {e}"))?;

    // tracing::info!(target: "spv", "wait {network} synced");
    // client.wait_synced().await;
    tracing::info!(target: "spv", "not synced");

    Ok(client)
}
