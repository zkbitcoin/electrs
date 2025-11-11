use anyhow::{Result, anyhow};
use serde_json::Value;
use bitcoincore_rpc::RpcApi;

pub struct PivxDaemon {
    pub inner: crate::daemon::Daemon,
}

impl PivxDaemon {
    pub fn new(inner: crate::daemon::Daemon) -> Self { Self { inner } }

    pub fn get_block_hash(&self, height: u32) -> Result<String> {
        let v: Value = self.inner.rpc.call("getblockhash", &[serde_json::json!(height)])?;
        Ok(v.as_str().ok_or_else(|| anyhow!("bad getblockhash"))?.to_string())
    }

    pub fn get_block_verbose(&self, block_hash: &str) -> Result<Value> {
        let v: Value = self.inner.rpc.call(
            "getblock",
            &[serde_json::json!(block_hash), serde_json::json!(2)],
        )?;
        Ok(v)
    }

    pub fn get_raw_transaction_verbose(&self, txid: &str) -> Result<Value> {
        let v: Value = self.inner.rpc.call(
            "getrawtransaction",
            &[serde_json::json!(txid), serde_json::json!(true)],
        )?;
        Ok(v)
    }
}
