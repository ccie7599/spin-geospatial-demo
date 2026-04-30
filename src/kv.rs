// ============================================================
// kv.rs — HTTP-backed KV store, drop-in replacement for spin_sdk::key_value::Store
//
// Speaks the NATS-KV adapter API at the same lat/lon shape Spin KV did:
//   GET  /v1/kv/<bucket>/<key>  →  raw bytes (404 if absent)
//   PUT  /v1/kv/<bucket>/<key>  →  raw bytes
//
// We deliberately use only get/set (no prefix scan, increment, CAS, history,
// or watch) so the architecture story remains "just a key-value store" and
// the only thing that changes is which KV is on the other end of the call.
// ============================================================

use spin_sdk::http::{Method, Request, Response, send};
use spin_sdk::variables;

pub struct KvStore {
    base: String,
    token: String,
    bucket: String,
}

impl KvStore {
    pub fn open_default() -> anyhow::Result<Self> {
        let base = variables::get("kv_endpoint")
            .map_err(|e| anyhow::anyhow!("kv_endpoint variable not set: {e}"))?;
        let token = variables::get("kv_token")
            .map_err(|e| anyhow::anyhow!("kv_token variable not set: {e}"))?;
        let bucket = variables::get("kv_bucket")
            .map_err(|e| anyhow::anyhow!("kv_bucket variable not set: {e}"))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token,
            bucket,
        })
    }

    pub async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let url = format!("{}/v1/kv/{}/{}", self.base, self.bucket, key);
        let req = Request::builder()
            .method(Method::Get)
            .uri(url)
            .header("authorization", format!("Bearer {}", self.token))
            .body(Vec::<u8>::new())
            .build();
        let resp: Response = send(req).await?;
        let status = u16::from(*resp.status());
        match status {
            200 => Ok(Some(resp.into_body())),
            404 => Ok(None),
            s => Err(anyhow::anyhow!("kv get {key} -> {s}")),
        }
    }

    pub async fn set(&self, key: &str, value: &[u8]) -> anyhow::Result<()> {
        let url = format!("{}/v1/kv/{}/{}", self.base, self.bucket, key);
        let req = Request::builder()
            .method(Method::Put)
            .uri(url)
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", "application/octet-stream")
            .body(value.to_vec())
            .build();
        let resp: Response = send(req).await?;
        let status = u16::from(*resp.status());
        if !(200..300).contains(&status) {
            return Err(anyhow::anyhow!("kv set {key} -> {status}"));
        }
        Ok(())
    }
}
