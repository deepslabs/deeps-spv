#[allow(unused)]
use bitcoin::consensus::serialize;
use rocksdb::{self};
use serde::{Deserialize, Serialize};

use crate::{
    header::{BlockEntry, HeaderEntry, HeaderList},
    utils::{deserialize_little, full_hash, serialize_little, Bytes, FullHash},
};
use bitcoin::block::Header as BlockHeader;
use std::{collections::HashMap, path::Path};

use bitcoin::{consensus::deserialize, BlockHash};
use std::{collections::HashSet, sync::RwLock};

/// Each version will break any running instance with a DB that has a differing version.
/// It will also break if light mode is enabled or disabled.
// 1 = Original DB (since fork from Blockstream)
// 2 = Add tx position to TxHistory rows and place Spending before Funding
static DB_VERSION: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DBRow {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

pub struct ScanIterator<'a> {
    prefix: Vec<u8>,
    iter: rocksdb::DBIterator<'a>,
    done: bool,
}

impl Iterator for ScanIterator<'_> {
    type Item = DBRow;

    fn next(&mut self) -> Option<DBRow> {
        if self.done {
            return None;
        }
        let (key, value) = self.iter.next().map(Result::ok)??;
        if !key.starts_with(&self.prefix) {
            self.done = true;
            return None;
        }
        Some(DBRow {
            key: key.to_vec(),
            value: unseal_data(value.to_vec()),
        })
    }
}

pub struct ReverseScanIterator<'a> {
    prefix: Vec<u8>,
    iter: rocksdb::DBRawIterator<'a>,
    done: bool,
}

impl Iterator for ReverseScanIterator<'_> {
    type Item = DBRow;

    fn next(&mut self) -> Option<DBRow> {
        if self.done || !self.iter.valid() {
            return None;
        }

        let key = self.iter.key().unwrap();
        if !key.starts_with(&self.prefix) {
            self.done = true;
            return None;
        }

        let row = DBRow {
            key: key.into(),
            value: unseal_data(self.iter.value().unwrap().to_vec()),
        };

        self.iter.prev();

        Some(row)
    }
}

#[derive(Debug)]
pub struct DB {
    db: rocksdb::DB,
}

#[derive(Copy, Clone, Debug)]
pub enum DBFlush {
    Disable,
    Enable,
}

#[allow(dead_code)]
impl DB {
    pub fn open(path: &Path) -> DB {
        let db = DB {
            db: open_raw_db(path),
        };
        db.verify_compatibility();
        db
    }

    pub fn full_compaction(&self) {
        // TODO: make sure this doesn't fail silently
        tracing::info!(target: "spv", "starting full compaction on {:?}", self.db);
        self.db.compact_range(None::<&[u8]>, None::<&[u8]>);
        tracing::info!(target: "spv", "finished full compaction on {:?}", self.db);
    }

    pub fn enable_auto_compaction(&self) {
        let opts = [("disable_auto_compactions", "false")];
        self.db.set_options(&opts).unwrap();
    }

    pub fn raw_iterator(&self) -> rocksdb::DBRawIterator {
        self.db.raw_iterator()
    }

    pub fn iter_scan(&self, prefix: &[u8]) -> ScanIterator {
        ScanIterator {
            prefix: prefix.to_vec(),
            iter: self.db.prefix_iterator(prefix),
            done: false,
        }
    }

    pub fn iter_scan_from(&self, prefix: &[u8], start_at: &[u8]) -> ScanIterator {
        let iter = self.db.iterator(rocksdb::IteratorMode::From(
            start_at,
            rocksdb::Direction::Forward,
        ));
        ScanIterator {
            prefix: prefix.to_vec(),
            iter,
            done: false,
        }
    }

    pub fn iter_scan_reverse(&self, prefix: &[u8], prefix_max: &[u8]) -> ReverseScanIterator {
        let mut iter = self.db.raw_iterator();
        iter.seek_for_prev(prefix_max);

        ReverseScanIterator {
            prefix: prefix.to_vec(),
            iter,
            done: false,
        }
    }

    pub fn write(&self, mut rows: Vec<DBRow>, flush: DBFlush) {
        tracing::info!(target: "spv",
            "writing {} rows to {:?}, flush={:?}",
            rows.len(),
            self.db,
            flush
        );
        rows.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        let mut batch = rocksdb::WriteBatch::default();
        for row in rows {
            batch.put(&row.key, seal_data(row.value));
        }
        let do_flush = match flush {
            DBFlush::Enable => true,
            DBFlush::Disable => false,
        };
        let mut opts = rocksdb::WriteOptions::new();
        opts.set_sync(do_flush);
        opts.disable_wal(!do_flush);
        self.db.write_opt(batch, &opts).unwrap();
    }

    pub fn flush(&self) {
        self.db.flush().unwrap();
    }

    pub fn put(&self, key: &[u8], value: &[u8]) {
        self.db.put(key, seal_data(value.to_vec())).unwrap();
    }

    pub fn put_sync(&self, key: &[u8], value: &[u8]) {
        let mut opts = rocksdb::WriteOptions::new();
        opts.set_sync(true);
        self.db
            .put_opt(key, seal_data(value.to_vec()), &opts)
            .unwrap();
    }

    pub fn get(&self, key: &[u8]) -> Option<Bytes> {
        self.db.get(key).unwrap().map(|v| unseal_data(v))
    }

    fn verify_compatibility(&self) {
        let compatibility_bytes = serialize_little(&DB_VERSION).unwrap();

        // if config.light_mode {
        //     compatibility_bytes.push(1);
        // }

        match self.get(b"V") {
            None => self.put(b"V", &compatibility_bytes),
            Some(ref x) if x != &compatibility_bytes => {
                panic!("Incompatible database found. Please reindex.")
            }
            Some(_) => (),
        }
    }
}

pub fn open_raw_db<T: rocksdb::ThreadMode>(path: &Path) -> rocksdb::DBWithThreadMode<T> {
    tracing::info!(target: "spv", "opening DB at {:?}", path);
    let mut db_opts = rocksdb::Options::default();
    db_opts.create_if_missing(true);
    db_opts.set_max_open_files(10_000); // TODO: make sure to `ulimit -n` this process correctly
    db_opts.set_compaction_style(rocksdb::DBCompactionStyle::Level);
    db_opts.set_compression_type(rocksdb::DBCompressionType::None);
    db_opts.set_target_file_size_base(0x16000000);
    db_opts.set_write_buffer_size(0x16000000);
    db_opts.set_disable_auto_compactions(true); // for initial bulk load

    // db_opts.set_advise_random_on_open(???);
    db_opts.set_compaction_readahead_size(1 << 20);
    db_opts.increase_parallelism(2);

    // let mut block_opts = rocksdb::BlockBasedOptions::default();
    // block_opts.set_block_size(???);

    rocksdb::DBWithThreadMode::<T>::open(&db_opts, path).expect("failed to open RocksDB")
}

#[derive(Serialize, Deserialize)]
struct BlockKey {
    code: u8,
    hash: FullHash,
}

pub struct BlockRow {
    key: BlockKey,
    value: Bytes, // serialized output
}

#[allow(dead_code)]
impl BlockRow {
    pub fn new_header(block_entry: &BlockEntry) -> BlockRow {
        BlockRow {
            key: BlockKey {
                code: b'B',
                hash: full_hash(&block_entry.entry.hash()[..]),
            },
            value: serialize(&block_entry.block.header),
        }
    }

    pub fn first_sync_header(header_entry: &HeaderEntry) -> BlockRow {
        BlockRow {
            key: BlockKey {
                code: b'b',
                hash: full_hash(header_entry.hash().as_ref()),
            },
            value: serialize(&header_entry.header()),
        }
    }

    pub fn new_done(hash: FullHash) -> BlockRow {
        BlockRow {
            key: BlockKey { code: b'D', hash },
            value: vec![],
        }
    }

    fn header_sync_filter() -> Bytes {
        b"b".to_vec()
    }

    fn header_filter() -> Bytes {
        b"B".to_vec()
    }

    fn txids_key(hash: FullHash) -> Bytes {
        [b"X", &hash[..]].concat()
    }

    fn meta_key(hash: FullHash) -> Bytes {
        [b"M", &hash[..]].concat()
    }

    fn done_filter() -> Bytes {
        b"D".to_vec()
    }

    fn into_row(self) -> DBRow {
        DBRow {
            key: serialize_little(&self.key).unwrap(),
            value: self.value,
        }
    }

    fn from_row(row: DBRow) -> Self {
        BlockRow {
            key: deserialize_little(&row.key).unwrap(),
            value: row.value,
        }
    }
}

pub struct Store {
    sync_db: DB,
    pub synced_blockhashes: RwLock<HashSet<BlockHash>>,
    pub sync_headers: RwLock<HeaderList>,
}

impl Store {
    pub fn open(path: &Path) -> Self {
        let sync_db = DB::open(&path.join("sync"));
        let synced_blockhashes = load_blockhashes(&sync_db, &BlockRow::done_filter());
        tracing::info!(target: "spv", "{} blocks were synced", synced_blockhashes.len());

        let sync_headers = if let Some(tip_hash) = sync_db.get(b"t") {
            let tip_hash = deserialize(&tip_hash).expect("invalid chain tip in `t`");
            let headers_map = load_sync_blockheaders(&sync_db);
            tracing::info!(target: "spv",
                "{} headers were loaded, tip at {:?}",
                headers_map.len(),
                tip_hash
            );
            HeaderList::new(headers_map, tip_hash)
        } else {
            HeaderList::empty()
        };

        Store {
            sync_db,
            synced_blockhashes: RwLock::new(synced_blockhashes),
            sync_headers: RwLock::new(sync_headers),
        }
    }

    #[allow(dead_code)]
    pub async fn reload_store(&self) {
        let sync_db = &self.sync_db;
        tracing::info!(target: "spv", "start reload");

        let mut reload = crate::RELOAD.write().await;
        *reload = true;
        drop(reload);
        tracing::info!(target: "spv", "set RELOAD to true");

        let synced_blockhashes = load_blockhashes(sync_db, &BlockRow::done_filter());
        tracing::info!(target: "spv", "reload {} blocks were synced", synced_blockhashes.len());

        let sync_headers = if let Some(tip_hash) = sync_db.get(b"t") {
            let tip_hash = deserialize(&tip_hash).expect("invalid chain tip in `t`");
            let headers_map = load_sync_blockheaders(sync_db);
            tracing::info!(target: "spv",
                "reload {} headers were loaded, tip at {:?}",
                headers_map.len(),
                tip_hash
            );
            HeaderList::new(headers_map, tip_hash)
        } else {
            HeaderList::empty()
        };
        tracing::info!(target: "spv", "reload {} sync_headers were synced", sync_headers.len());

        let mut synced_blockhashes_reload = self.synced_blockhashes.write().unwrap();
        let mut sync_headers_reload = self.sync_headers.write().unwrap();
        tracing::info!(target: "spv", "reload get write locks");

        *synced_blockhashes_reload = synced_blockhashes;
        *sync_headers_reload = sync_headers;
        drop(synced_blockhashes_reload);
        drop(sync_headers_reload);
        tracing::info!(target: "spv", "reload finished drop locks");
        let mut reload = crate::RELOAD.write().await;
        *reload = false;
        drop(reload);
        tracing::info!(target: "spv", "set RELOAD to false");
    }

    pub fn sync_db(&self) -> &DB {
        &self.sync_db
    }

    #[allow(dead_code)]
    pub fn full_compaction(&self) {
        self.sync_db.full_compaction();
        self.sync_db.enable_auto_compaction();
    }
}

fn load_blockhashes(db: &DB, prefix: &[u8]) -> HashSet<BlockHash> {
    db.iter_scan(prefix)
        .map(BlockRow::from_row)
        .map(|r| deserialize(&r.key.hash).expect("failed to parse BlockHash"))
        .collect()
}

fn load_sync_blockheaders(db: &DB) -> HashMap<BlockHash, BlockHeader> {
    db.iter_scan(&BlockRow::header_sync_filter())
        .map(BlockRow::from_row)
        .map(|r| {
            let key: BlockHash = deserialize(&r.key.hash).expect("failed to parse BlockHash");
            let value: BlockHeader = deserialize(&r.value).expect("failed to parse BlockHeader");
            (key, value)
        })
        .collect()
}

pub fn add_sync_headers(header_entries: &[HeaderEntry], _thread: usize) -> Vec<DBRow> {
    let mut rows = vec![];
    for v in header_entries {
        let blockhash = full_hash(&v.hash()[..]);

        rows.push(BlockRow::first_sync_header(v).into_row());
        rows.push(BlockRow::new_done(blockhash).into_row());
    }

    rows
    /*
    let mut hs = Vec::new();

    for v in header_entries.chunks(usize::max(header_entries.len() / thread, 1)) {
        let v = v.to_owned();
        let h = thread::spawn(|| {
            let mut rows = vec![];
            v.into_iter().for_each(|h| {
                let blockhash = full_hash(&h.hash()[..]);

                rows.push(BlockRow::first_sync_header(&h).into_row());
                rows.push(BlockRow::new_done(blockhash).into_row()); // mark block as "added"
            });
            rows
        });
        hs.push(h);
    }
    hs.into_iter()
        .map(|h| h.join().unwrap())
        .flatten()
        .collect()
         */
}

pub fn seal_data(value: Vec<u8>) -> Vec<u8> {
    sxn_rsv::sealing(value).unwrap()
}

pub fn unseal_data(value: Vec<u8>) -> Vec<u8> {
    sxn_rsv::unsealing(value).unwrap()
}
