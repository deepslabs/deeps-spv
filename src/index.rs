use std::sync::Arc;

use bincode::deserialize;
use bitcoin::{consensus::serialize, BlockHash};
use serde::{Deserialize, Serialize};

use crate::{
    config::ConfigCli,
    daemon::Daemon,
    header::HeaderEntry,
    store::{add_sync_headers, DBFlush, Store},
};

#[allow(dead_code)]
pub struct Indexer {
    store: Arc<Store>,
    flush: DBFlush,
    thread: usize,
}

#[allow(dead_code)]
impl Indexer {
    pub fn open(store: Arc<Store>, config: &ConfigCli) -> Self {
        Indexer {
            store,
            flush: DBFlush::Disable,
            thread: config.thread,
        }
    }

    fn add_sync_block_header(&self, headers: &[HeaderEntry]) {
        tracing::debug!(target: "spv", "Adding {} blocks to headers from rpc", headers.len());
        let rows = add_sync_headers(headers, self.thread);
        for row in rows.chunks(50000) {
            self.store.sync_db().write(row.to_vec(), DBFlush::Enable);
        }
        self.store
            .synced_blockhashes
            .write()
            .unwrap()
            .extend(headers.iter().map(|b| {
                if b.height() % 10_000 == 0 {
                    tracing::info!(target: "spv", "rpc sync header is up to height={}", b.height());
                }
                b.hash()
            }));
    }

    fn sync_headers_to_add(&self, new_headers: &[HeaderEntry]) -> Vec<HeaderEntry> {
        let added_blockhashes = self.store.synced_blockhashes.read().unwrap();
        new_headers
            .iter()
            .filter(|e| !added_blockhashes.contains(e.hash()))
            .cloned()
            .collect()
    }

    pub fn sync_headers(&self, daemon: &Daemon, tip: &BlockHash) -> Result<(), String> {
        // sync download new headers and index it and fulsh it to db
        let sync_headers = self.store.sync_headers.read().unwrap();
        tracing::debug!(target: "spv", "rpc {:?} headers already synced", sync_headers.len());
        let new_headers = daemon.get_new_headers(&sync_headers, tip)?;
        let sync_header_result = sync_headers.order(new_headers);
        drop(sync_headers);
        let to_add = self.sync_headers_to_add(&sync_header_result);
        tracing::debug!(target: "spv", "rpc {:?} headers to add", to_add.len());
        self.add_sync_block_header(&to_add);
        self.store.sync_db().put_sync(b"t", &serialize(&tip));
        {
            let mut headers_sync = self.store.sync_headers.write().unwrap();
            headers_sync.apply(to_add);
        }
        let headers_sync = self.store.sync_headers.read().unwrap();
        if tip != headers_sync.tip() {
            tracing::debug!(target: "spv", "tip {} headers_sync.tip() {} not equal", tip, headers_sync.tip());
            //assert_eq!(tip, headers_sync.tip());
            drop(headers_sync);
            crate::RUNTIME.block_on(async move {
                self.store.reload_store().await;
            });

            return Err("tip  headers_sync.tip() not equal, try again".to_string());
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn sync_headers_with_api_check(
        &self,
        daemon: &Daemon,
        tip: &BlockHash,
        _apis: Vec<Vec<u8>>,
    ) -> Result<(), String> {
        // sync download new headers and index it and fulsh it to db
        let sync_headers = self.store.sync_headers.read().unwrap();
        tracing::debug!(target: "spv", "rpc {:?} headers already synced", sync_headers.len());
        let new_headers = daemon.get_new_headers(&sync_headers, tip)?;
        let sync_header_result = sync_headers.order(new_headers);
        drop(sync_headers);
        let to_add = self.sync_headers_to_add(&sync_header_result);
        tracing::debug!(target: "spv", "rpc {:?} headers to add", to_add.len());
        self.add_sync_block_header(&to_add);
        self.store.sync_db().put_sync(b"t", &serialize(&tip));
        let mut headers_sync = self.store.sync_headers.write().unwrap();
        headers_sync.apply(to_add);
        assert_eq!(tip, headers_sync.tip());
        drop(headers_sync);
        Ok(())
    }

    pub fn full_compaction(&self) {
        self.store.full_compaction();
    }
}

// {
//  "name": "mempool",
// 	"method": "get",
// 	"url": "https://mempool.space/testnet/api/block/{replace}/header",
// 	"replace": "blockhash",
// 	"data": "none",
//  "return": "string",
//  "returnvaluekey": "none"
// }
#[derive(Serialize, Deserialize)]
pub struct Api {
    name: String,
    method: String,
    url: String,
    replace: String,
    data: String,
    returntype: String,
    returnvaluekey: String,
}

#[allow(dead_code)]
pub fn api_check(latest_headers: Vec<HeaderEntry>, apis: Vec<Vec<u8>>) -> Result<(), String> {
    for h in latest_headers {
        let hash = h.hash().to_string();
        let header = hex::encode(serialize(h.header()));

        for api_vec in apis.clone() {
            let api: Api = deserialize(&api_vec).map_err(|e| format!("bad api {e}"))?;
            let mut url = api.url;
            url = url.replace("{replace}", &hash);

            let value = if api.data != "none" {
                // parse data & send post
                serde_json::Value::default()
            } else {
                //send get
                crate::utils::request_get(url).map_err(|e| e.to_string())?
            };

            let api_header = if api.returntype == "string" {
                value.to_string()
            } else {
                // value.get(" api.rerutnvalue ")
                String::new()
            };

            if header != api_header {
                return Err(format!(
                    "mempool api check failed rpc[{header}] mempool[{}]",
                    value
                ));
            }
        }
    }
    Ok(())
}
