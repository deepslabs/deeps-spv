use bitcoin::{BlockHash, Txid};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Copy, Clone, PartialEq, Hash, Serialize, Ord, PartialOrd, Eq)]
pub enum Network {
    Bitcoin,
    Testnet,
    Testnet4,
    Fractal,
    Regtest,
    Signet,
    DogecoinMainnet,
    DogecoinTestnet,
    DogecoinRegtest,
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Bitcoin => "bitcoin",
            Self::Testnet => "testnet",
            Self::Testnet4 => "testnet4",
            Self::Fractal => "fractal",
            Self::DogecoinMainnet => "dogecoin_mainnet",
            Self::DogecoinTestnet => "dogecoin_testnet",
            Self::DogecoinRegtest => "dogecoin_regtest",
            Self::Regtest => "regtest",
            Self::Signet => "signet",
        };
        s.fmt(f)
    }
}

impl std::str::FromStr for Network {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bitcoin" => Ok(Self::Bitcoin),
            "testnet" => Ok(Self::Testnet),
            "testnet4" => Ok(Self::Testnet4),
            "fractal" => Ok(Self::Fractal),
            "regtest" => Ok(Self::Regtest),
            "signet" => Ok(Self::Signet),
            "dogecoin_mainnet" => Ok(Self::DogecoinMainnet),
            "dogecoin_testnet" => Ok(Self::DogecoinTestnet),
            "dogecoin_regtest" => Ok(Self::DogecoinRegtest),
            _ => Err(format!("Unknown log level: {s}")),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum MerkleProofParamChain {
    BitCoin,
    DogeCoin,
}

pub struct MerkleProofParam {
    pub txids: Vec<Txid>,
    pub blockhash: Option<BlockHash>,
    pub chain: MerkleProofParamChain,
}

impl MerkleProofParam {
    pub fn new(
        txids: Vec<Txid>,
        blockhash: Option<BlockHash>,
        chain: MerkleProofParamChain,
    ) -> Self {
        Self {
            txids,
            blockhash,
            chain,
        }
    }

    pub fn to_string(&self) -> Value {
        let txids_string: Vec<String> = self.txids.iter().map(|txid| txid.to_string()).collect();
        let blockhash_string = match self.blockhash {
            Some(hash) => hash.to_string(),
            None => {
                return json!([txids_string]);
            }
        };
        json!([txids_string, blockhash_string])
    }
}
