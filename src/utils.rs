use arrayref::array_ref;
use bincode::Options;
use lazy_static::lazy_static;
use reqwest::{blocking::Client, Url};
use serde_json::Value;
use std::{collections::BTreeMap, hash::Hash, sync::Arc};
use tokio::sync::RwLock;

pub type Bytes = Vec<u8>;
pub type FullHash = [u8; 32];
const HASH_LEN: usize = 32;

lazy_static! {
    static ref HTTP_CLIENT: Client = Client::new();
}

pub fn full_hash(hash: &[u8]) -> FullHash {
    *array_ref![hash, 0, HASH_LEN]
}

pub fn request(addr: String, auth: String, req: &Value) -> Result<Value, String> {
    let url = Url::parse(&addr).unwrap();

    let response: String = HTTP_CLIENT
        .post(url)
        .header("Content-Type", "application/json")
        .header(reqwest::header::AUTHORIZATION, auth)
        .body(req.to_string())
        .send()
        .map_err(|e| format!("failed to get response {e:?}"))?
        .text()
        .map_err(|e| format!("failed to get payload {e:?}"))?;

    let result: Value = serde_json::from_str(&response).map_err(|e| format!("json error {e}"))?;
    Ok(result)
}

#[allow(dead_code)]
pub fn request_get(addr: String) -> Result<Value, String> {
    let url = Url::parse(&addr).unwrap();

    let response: String = HTTP_CLIENT
        .get(url)
        .header("Content-Type", "application/json")
        .send()
        .map_err(|e| format!("failed to get response {e}"))?
        .text()
        .map_err(|e| format!("failed to get payload {e}"))?;

    let result: Value = serde_json::from_str(&response).map_err(|e| format!("json error {e}"))?;
    Ok(result)
}

#[inline]
fn options() -> impl Options {
    bincode::options()
        .with_fixint_encoding()
        .with_no_limit()
        .allow_trailing_bytes()
}

#[inline]
fn little_endian() -> impl Options {
    options().with_little_endian()
}

pub fn serialize_little<T>(value: &T) -> Result<Vec<u8>, bincode::Error>
where
    T: ?Sized + serde::Serialize,
{
    little_endian().serialize(value)
}

pub fn deserialize_little<'a, T>(bytes: &'a [u8]) -> Result<T, bincode::Error>
where
    T: serde::Deserialize<'a>,
{
    little_endian().deserialize(bytes)
}

pub struct SimpleCache<K, V> {
    map: Arc<RwLock<BTreeMap<K, Arc<RwLock<V>>>>>,
    size: usize,
}

impl<K, V> SimpleCache<K, V>
where
    K: Eq + Hash + Clone + Ord,
    V: Clone,
{
    pub fn new(size: usize) -> Self {
        SimpleCache {
            map: Arc::new(RwLock::new(BTreeMap::new())),
            size,
        }
    }

    // insert on update k-v
    pub async fn insert(&self, key: K, value: V) {
        let mut map = self.map.write().await;

        if map.len() >= self.size {
            if let Some((oldest_key, _)) = map.clone().into_iter().next().clone() {
                map.remove(&oldest_key);
            }
        }

        map.insert(key.clone(), Arc::new(RwLock::new(value)));
        drop(map);
    }

    // get of k
    pub async fn get(&self, key: &K) -> Option<V> {
        let map = self.map.read().await;
        let result = map.get(key).map(|arc_rwlock| Arc::clone(arc_rwlock));
        drop(map);

        match result {
            Some(arc_rwlock) => Some(arc_rwlock.read().await.clone()),
            None => None,
        }
    }

    // update v of k
    #[allow(dead_code)]
    pub async fn write(&self, key: &K, new_value: V) {
        let map = self.map.read().await;
        if let Some(arc_rwlock) = map.get(key) {
            let mut write_guard = arc_rwlock.write().await;
            *write_guard = new_value;
            drop(write_guard);
        } else {
            drop(map);
        }
    }

    // remove k-v
    pub async fn remove(&self, key: &K) {
        let mut map = self.map.write().await;
        map.remove(key);
        drop(map);
    }
}
