use std::sync::Arc;

use crate::chains::sol::SolClient;
use crate::types::MerkleProofParamChain;
use crate::{daemon::Daemon, store::Store, types::MerkleProofParam, utils::SimpleCache};
use bitcoin::{
    block::Header,
    consensus::{deserialize, serialize},
    MerkleBlock,
};
use bitcoinnakamoto::BlockHeader;
use bitcoinnakamoto::{
    self,
    consensus::{deserialize as deserialize_old, serialize as serialize_old},
};
use helios::{
    ethereum::{database::ConfigDB, EthereumClient},
    opstack::OpStackClient,
};
use nakamoto::client::handle::Handle as _;
use nakamoto::client::Handle;
use nakamoto::net::poll::Waker;
use serde_json::Value;

#[allow(clippy::too_many_arguments)]
pub struct ChainsState {
    pub store: Arc<Store>,
    pub daemon: Option<Arc<Daemon>>,
    pub doge_daemon: Option<Arc<Daemon>>,
    pub handle: Option<Arc<Handle<Waker>>>,
    pub doge_handle: Option<Arc<Handle<Waker>>>,
    pub helios_client: Option<Arc<EthereumClient<ConfigDB>>>,
    pub helios_op_client: Option<Arc<OpStackClient>>,
    pub response_cache: Arc<SimpleCache<Vec<u8>, Value>>,
    pub sol_client: Option<Arc<SolClient>>,
}

impl ChainsState {
    pub fn new(
        store: Arc<Store>,
        daemon: Option<Arc<Daemon>>,
        doge_daemon: Option<Arc<Daemon>>,
        handle: Option<Arc<Handle<Waker>>>,
        doge_handle: Option<Arc<Handle<Waker>>>,
        helios_client: Option<Arc<EthereumClient<ConfigDB>>>,
        helios_op_client: Option<Arc<OpStackClient>>,
        sol_client: Option<Arc<SolClient>>,
        response_cache: Arc<SimpleCache<Vec<u8>, Value>>,
    ) -> Self {
        Self {
            store,
            daemon,
            doge_daemon,
            handle,
            doge_handle,
            helios_client,
            helios_op_client,
            sol_client,
            response_cache,
        }
    }

    pub async fn fetch_proof_and_verify(
        &self,
        param: &MerkleProofParam,
        response_cache: Arc<SimpleCache<Vec<u8>, Value>>,
    ) -> Result<Vec<u8>, String> {
        let mb = match param.chain {
            MerkleProofParamChain::BitCoin => self
                .daemon
                .as_ref()
                .unwrap()
                .gettxoutproof(param)
                .map_err(|e| format!("gettxoutproof error {e}"))?,
            MerkleProofParamChain::DogeCoin => self
                .doge_daemon
                .as_ref()
                .unwrap()
                .gettxoutproof(param)
                .map_err(|e| format!("doge gettxoutproof error {e}"))?,
        };

        self.verify_txid(&mb, param, response_cache).await
    }

    pub async fn verify_txid(
        &self,
        mb: &MerkleBlock,
        param: &MerkleProofParam,
        response_cache: Arc<SimpleCache<Vec<u8>, Value>>,
    ) -> Result<Vec<u8>, String> {
        let mb_blockhash = mb.header.block_hash();

        // Verify blockhash if provided in the parameters
        if let Some(param_blockhash) = param.blockhash {
            if param_blockhash != mb_blockhash {
                return Err(format!(
                    "Blockhash mismatch: expected {}, got {}",
                    param_blockhash, mb_blockhash
                ));
            }
        }

        // Extract transaction matches and verify inclusion
        let mut matches = vec![];
        let txroot = mb
            .txn
            .extract_matches(&mut matches, &mut vec![])
            .map_err(|e| {
                format!(
                    "Transaction IDs {:?} not included in block {}: {}",
                    param.txids, mb_blockhash, e
                )
            })?;

        // Convert blockhash to the old format for P2P query
        let blockhash_old: bitcoinnakamoto::BlockHash =
            deserialize_old(&serialize(&mb_blockhash)).unwrap();

        let header_p2p: BlockHeader = match response_cache.get(&blockhash_old.to_vec()).await {
            Some(block) => {
                let h = serde_json::from_value(block)
                    .map_err(|e| format!("deserileze P2P header error: {}", e))?;
                h
            }
            None => {
                let handle = match param.chain {
                    MerkleProofParamChain::BitCoin => self.handle.as_ref().unwrap(),
                    MerkleProofParamChain::DogeCoin => self.doge_handle.as_ref().unwrap(),
                };
                let (_, header_p2p) = handle
                    .get_block(&blockhash_old)
                    .map_err(|e| format!("{:?} get P2P header error: {}", param.chain, e))?
                    .ok_or("block not found".to_string())?;

                let header_str = serde_json::to_value(&header_p2p)
                    .map_err(|e| format!("{:?} P2P to_value error: {}", param.chain, e))?;
                response_cache
                    .insert(blockhash_old.to_vec(), header_str)
                    .await;

                header_p2p
            }
        };

        // Convert P2P header to the current format
        let header_p2p: Header = deserialize(&serialize_old(&header_p2p)).unwrap();

        // Verify cached header if applicable
        if false {
            // !(self.only_p2p || is_doge)
            let cached_headers = self.store.sync_headers.read().unwrap();
            let cached_header = cached_headers
                .header_by_blockhash(&mb_blockhash)
                .ok_or(format!("No header cached for block {}", mb_blockhash))?
                .clone();

            if *cached_header.header() != header_p2p {
                return Err(format!(
                    "Cached header {:?} does not match P2P header {:?} for block {}",
                    cached_header.header(),
                    header_p2p,
                    mb_blockhash
                ));
            }
        }

        // Verify Merkle root
        if txroot != header_p2p.merkle_root {
            return Err(format!(
                "Merkle root mismatch: expected {}, got {} for block {}",
                txroot, header_p2p.merkle_root, mb_blockhash
            ));
        }

        // Return serialized Merkle block
        Ok(bitcoin::consensus::serialize(&mb))
    }
}

#[test]
pub fn merkle_block_test() {
    use std::str::FromStr;

    use bitcoin::{consensus::deserialize, BlockHash, MerkleBlock, Txid};
    let mb_hex = hex::decode("000000305fad602fba2e8f80479fe1755428bf5c0101fb0e76759cf0364fa07b329e6236938f812c2ff7cc7d1097548a0f56a6817bb9cb9853ca82cccafbe7b6caa37863e480d466ffff7f20010000000100000001938f812c2ff7cc7d1097548a0f56a6817bb9cb9853ca82cccafbe7b6caa378630101");
    let mb: MerkleBlock = deserialize(&mb_hex.unwrap()).unwrap();

    assert_eq!(
        mb.header.block_hash(),
        BlockHash::from_str("0b93ed09ec301ef95f349dec8b66bea53702b36383e72449ee5cae312894e24f")
            .unwrap()
    );

    let mut matches = vec![];
    let mut indexes = vec![];
    let txroot = mb.txn.extract_matches(&mut matches, &mut indexes).unwrap();

    assert_eq!(
        matches[0],
        Txid::from_str("6378a3cab6e7fbcacc82ca5398cbb97b81a6560f8a5497107dccf72f2c818f93").unwrap()
    );
    assert_eq!(indexes[0], 0u32);

    assert_eq!(txroot, mb.header.merkle_root);
}

#[test]
pub fn test_gettxoutproof_rpc() {
    use std::str::FromStr;

    use bitcoin::{BlockHash, Txid};

    let daemon = crate::daemon::Daemon::new(
        "127.0.0.1:18447".parse().unwrap(),
        String::from("prz:prz"),
        crate::types::Network::Regtest,
        false,
    )
    .unwrap();
    let tx =
        Txid::from_str("2518f2c6088e33465a2edfb7fe695db35be648d3c1290594428e8e5fd56a103d").unwrap();
    let txids = vec![tx];
    let blockhash =
        BlockHash::from_str("3a719bb7c2d1b8a818f34163a073c935c280ee00153bb7572354d74738c48e20")
            .unwrap();

    let para1 = crate::types::MerkleProofParam::new(
        txids.clone(),
        Some(blockhash),
        crate::types::MerkleProofParamChain::BitCoin,
    );
    let para2 = crate::types::MerkleProofParam::new(
        txids,
        None,
        crate::types::MerkleProofParamChain::BitCoin,
    );

    let data = daemon.gettxoutproof(&para1).unwrap();
    println!("{:?}", data);
    let data = daemon.gettxoutproof(&para2).unwrap();
    println!("{:?}", data);
}
