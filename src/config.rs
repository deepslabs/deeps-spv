use crate::types::Network;
use clap::arg;
use std::net::SocketAddr;

use clap::builder::TypedValueParser as _;
use clap::Parser;

#[derive(Debug, Clone, Parser)]
pub struct ConfigCli {
    // bitcoin config
    #[arg(
        long,
        default_value_t = Network::Testnet,
        value_parser = clap::builder::PossibleValuesParser::new(
            ["bitcoin","testnet","testnet4","fractal","regtest","signet"]
        )
            .map(|s| s.parse::<Network>().unwrap()),
    )]
    pub network_type: Network,
    #[arg(long, help = "bitcond rpc address")]
    pub daemon_rpc_addr: SocketAddr,
    #[arg(long)]
    pub cookie: Option<String>,
    // doge config
    #[arg(
        long,
        default_value_t = Network::DogecoinTestnet,
        value_parser = clap::builder::PossibleValuesParser::new(["dogecoin_mainnet","dogecoin_testnet", "dogecoin_regtest"])
            .map(|s| s.parse::<Network>().unwrap()),
    )]
    pub doge_network_type: Network,
    #[arg(long, help = "doge rpc address")]
    pub doge_daemon_rpc_addr: SocketAddr,
    #[arg(long)]
    pub doge_cookie: Option<String>,
    #[arg(long)]
    pub regtest_peer: Vec<SocketAddr>,
    // server & http port to listen
    #[arg(long)]
    pub http_addr: SocketAddr,
    // dir for store data
    #[arg(long, default_value_t = String::from("./db"))]
    pub store: String,
    // config for registeration to bool
    #[arg(long, default_value_t = String::from("http://127.0.0.1:9933"))]
    pub subclient_url: String,
    #[arg(long)]
    pub device_owner: String,
    #[arg(long)]
    pub watcher_device_id: String,
    #[arg(long, default_value_t = String::from("0x1234"))]
    pub relate_device_id_test: String,
    #[arg(long, default_value_t = false)]
    pub sgx_enable: bool,
    #[arg(long, default_value_t = 30)]
    pub thread: usize,
    // eth config
    #[arg(long, default_value_t = String::from("https://www.lightclientdata.org"))]
    pub consensus_rpc: String,
    #[arg(long, default_value_t = String::from("https://eth-mainnet.g.alchemy.com/v2/"))]
    pub execution_rpc: String,
    #[arg(long, default_value_t = String::from("mainnet"))]
    pub eth_network: String,
    // op config
    #[arg(long, default_value_t = String::from("https://op-mainnet.operationsolarstorm.org"))]
    pub op_consensus_rpc: String,
    #[arg(long, default_value_t = String::from("https://opt-mainnet.g.alchemy.com/v2/"))]
    pub op_execution_rpc: String,
    #[arg(long, default_value_t = String::from("op-mainnet"))]
    pub op_network: String,
    // sol config
    #[arg(long)]
    pub sol_judge_rpc: String,
    #[arg(long)]
    pub sol_extra_rpcs: String,
    // Flags controlling whether various features are enabled
    #[arg(long, default_value_t = false)]
    pub electrs_support: bool,
    #[arg(long, default_value_t = false)]
    pub disable_helios: bool,
    #[arg(long, default_value_t = false)]
    pub btc_only_p2p: bool,
    #[arg(long, default_value_t = String::from("btc,doge,eth,op,sol"))]
    pub start_filter: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub configcli: ConfigCli,
    pub service: Service,
}

impl Config {
    pub fn from_args() -> Config {
        let configcli = ConfigCli::parse();
        Config {
            configcli: configcli.clone(),
            service: start_services(start_filter(configcli)),
        }
    }
}

fn start_filter(config: ConfigCli) -> Vec<String> {
    let config = config.start_filter;
    let start_service: Vec<String> = config.split(',').map(|v| v.to_string()).collect();
    start_service
}

#[derive(Debug, Copy, Clone, Default)]
pub struct Service {
    pub btc: bool,
    pub eth: bool,
    pub op: bool,
    pub doge: bool,
    pub sol: bool,
}

fn start_services(se: Vec<String>) -> Service {
    let mut service = Service::default();

    for s in se {
        match s.as_str() {
            "btc" => service.btc = true,
            "eth" => service.eth = true,
            "op" => service.op = true,
            "doge" => service.doge = true,
            "sol" => service.sol = true,
            _ => {
                tracing::info!("unknown service {s}")
            }
        }
    }
    service
}
