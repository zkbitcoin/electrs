use anyhow::{Context, Result};

use bitcoin::{consensus::deserialize, hashes::hex::FromHex};
use bitcoin::{Amount, BlockHash, Transaction, Txid};
use bitcoin::block::{Header as BlockHeader, Version};
use bitcoin::pow::CompactTarget;
use bitcoincore_rpc::{json, jsonrpc, Auth, Client, RpcApi};
use bitcoincore_rpc::json::GetNetworkInfoResult;
use bitcoin::hashes::Hash;
use crossbeam_channel::Receiver;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, value::RawValue, Value};

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::{
    chain::{Chain, NewHeader},
    config::Config,
    metrics::Metrics,
    p2p::Connection,
    signals::ExitFlag,
    types::SerBlock,
};

// --- PIVX integration imports ---
use log::{info, debug, warn};
use crate::chain_kind::{chain_from_env, ChainKind};

enum PollResult {
    Done(Result<()>),
    Retry,
}

fn debug_json_parse<T: serde::de::DeserializeOwned>(
    raw: serde_json::Value,
    context: &'static str,
) -> anyhow::Result<T> {
    match serde_json::from_value::<T>(raw.clone()) {
        Ok(parsed) => Ok(parsed),
        Err(e) => {
            use std::backtrace::Backtrace;
            use log::{error, debug};

            let bt = Backtrace::force_capture();
            let thread = std::thread::current()
                .name()
                .unwrap_or("unnamed-thread")
                .to_string();

            // Pretty-print the JSON (limited to 8 KiB for sanity)
            let raw_str = match serde_json::to_string_pretty(&raw) {
                Ok(s) if s.len() < 8192 => s,
                Ok(_) => "<JSON too large to display>".to_string(),
                Err(_) => "<unprintable JSON>".to_string(),
            };

            error!(
                "❌ JSON parse error in {} (thread: {}): {}\nOffending value:\n{}\nBacktrace:\n{:?}",
                context,
                thread,
                e,
                raw_str,
                bt
            );

            // Optional short line for immediate grep/debug visibility
            debug!("🔎 JSON parse error originated from {}", context);

            // Re-wrap the same error with anyhow context
            Err(anyhow::anyhow!(e).context(context))
        }
    }
}



fn rpc_poll(client: &mut Client, skip_block_download_wait: bool) -> PollResult {
    // Use unified chain-aware function
    match crate::daemon::Daemon::get_blockchain_info_patched_core(client) {
        Ok(info) => {
            if skip_block_download_wait {
                return PollResult::Done(Ok(()));
            }

            let left_blocks = info.headers.saturating_sub(info.blocks);
            if info.initial_block_download || left_blocks > 0 {
                info!(
                    "waiting for {} blocks to download{}",
                    left_blocks,
                    if info.initial_block_download { " (IBD)" } else { "" }
                );
                return PollResult::Retry;
            }

            PollResult::Done(Ok(()))
        }

        Err(err) => {
            // 🧠 Downcast from anyhow::Error → bitcoincore_rpc::Error
            if let Some(inner) = err.downcast_ref::<bitcoincore_rpc::Error>() {
                if let Some(e) = extract_bitcoind_error(inner) {
                    if e.code == -28 {
                        debug!("waiting for RPC warmup: {}", e.message);
                        return PollResult::Retry;
                    }
                }
            }

            PollResult::Done(Err(err).context("daemon not available"))
        }
    }
}

fn read_cookie(path: &Path) -> Result<(String, String)> {
    // Load username and password from bitcoind cookie file:
    // * https://github.com/bitcoin/bitcoin/pull/6388/commits/71cbeaad9a929ba6a7b62d9b37a09b214ae00c1a
    // * https://bitcoin.stackexchange.com/questions/46782/rpc-cookie-authentication
    let mut file = File::open(path)
        .with_context(|| format!("failed to open bitcoind cookie file: {}", path.display()))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .with_context(|| format!("failed to read bitcoind cookie from {}", path.display()))?;

    let parts: Vec<&str> = contents.splitn(2, ':').collect();
    ensure!(
        parts.len() == 2,
        "failed to parse bitcoind cookie - missing ':' separator"
    );
    Ok((parts[0].to_owned(), parts[1].to_owned()))
}

fn rpc_connect(config: &Config) -> Result<Client> {
    let rpc_url = format!("http://{}", config.daemon_rpc_addr);
    // Allow RPC calls to take longer before timing out.
    // See https://github.com/romanz/electrs/issues/495 for more details.
    let builder = jsonrpc::simple_http::SimpleHttpTransport::builder()
        .url(&rpc_url)?
        .timeout(config.jsonrpc_timeout);
    let builder = match config.daemon_auth.get_auth() {
        Auth::None => builder,
        Auth::UserPass(user, pass) => builder.auth(user, Some(pass)),
        Auth::CookieFile(path) => {
            let (user, pass) = read_cookie(&path)?;
            builder.auth(user, Some(pass))
        }
    };
    Ok(Client::from_jsonrpc(jsonrpc::Client::with_transport(
        builder.build(),
    )))
}

pub fn pivx_rpc_from_config(cfg: &Config) -> bitcoincore_rpc::Client {
    let auth = cfg.daemon_auth.get_auth();
    let rpc_url = format!("http://{}", cfg.daemon_rpc_addr);
    Client::new(&rpc_url, auth).expect("PIVX: failed to connect to pivxd RPC")
}

pub struct Daemon {
    pub(crate) p2p: Mutex<Connection>,
    pub rpc: Client,
}

impl Daemon {

    // =====================================================
    // Override for all subsequent calls to get_blockchaininfo
    // =====================================================
    pub(crate) fn get_blockchain_info(&self) -> anyhow::Result<bitcoincore_rpc::json::GetBlockchainInfoResult> {
        let chain = crate::chain_kind::chain_from_env();
        if chain == crate::chain_kind::ChainKind::Pivx {
            return self.get_blockchain_info_patched();
        }
        Ok(self.rpc.get_blockchain_info()?)
    }

    /// Override for all subsequent getnetworkinfo() calls
    /// Uses PIVX-patched version if chain_from_env() == Pivx
    pub(crate) fn get_network_info(
        &self,
    ) -> anyhow::Result<bitcoincore_rpc::json::GetNetworkInfoResult> {
        let chain = crate::chain_kind::chain_from_env();
        if chain == crate::chain_kind::ChainKind::Pivx {
            return self.get_network_info_patched();
        }
        Ok(self.rpc.get_network_info()?)
    }

    pub(crate) fn get_network_info_patched(&self) -> Result<GetNetworkInfoResult> {
        Self::get_network_info_patched_with(&self.rpc)
    }

    pub(crate) fn get_network_info_patched_with(rpc: &Client) -> Result<GetNetworkInfoResult> {
        use log::{debug, info};
        use serde_json::Value;

        let chain = crate::chain_kind::chain_from_env();
        if chain != crate::chain_kind::ChainKind::Pivx {
            // Default Bitcoin path
            return Ok(rpc.get_network_info()?);
        }

        debug!("(patched_with) Entered PIVX get_network_info_patched_with()");

        let mut raw: Value = rpc.call("getnetworkinfo", &[])?;
        debug!("PIVX raw getnetworkinfo: {}", raw);

        // --- networks: ensure it's an ARRAY (Vec) ---
        if let Some(networks_val) = raw.get_mut("networks") {
            match networks_val {
                Value::Array(_) => {
                    // fine
                }
                Value::Object(obj) => {
                    // convert map -> array of values
                    let arr: Vec<Value> = obj.values().cloned().collect();
                    *networks_val = Value::Array(arr);
                    info!("PIVX: patched getnetworkinfo.networks from map → array");
                }
                _ => {
                    *networks_val = Value::Array(vec![]);
                    info!("PIVX: normalized getnetworkinfo.networks to empty array");
                }
            }
        } else {
            raw.as_object_mut()
                .expect("PIVX: expected object")
                .insert("networks".to_string(), Value::Array(vec![]));
            info!("PIVX: injected missing field `networks: []`");
        }

        // --- localaddresses must be array ---
        {
            let obj = raw.as_object_mut().expect("PIVX: expected object");
            if !obj.get("localaddresses").map(|v| v.is_array()).unwrap_or(false) {
                obj.insert("localaddresses".to_string(), Value::Array(vec![]));
                debug!("PIVX: normalized `localaddresses` → []");
            }
        }

        // --- inject other fields that GetNetworkInfoResult expects ---
        {
            let obj = raw.as_object_mut().expect("PIVX: expected object");
            let defaults: [(&str, Value); 5] = [
                ("localrelay", Value::Bool(true)),
                ("incrementalfee", Value::from(0.0001_f64)),
                ("localservicesnames", Value::Array(vec![])),
                ("connections_in", Value::from(0)),
                ("connections_out", Value::from(0)),
            ];
            for (k, v) in defaults {
                if !obj.contains_key(k) {
                    obj.insert(k.to_string(), v);
                    debug!("PIVX: injected missing field `{}`", k);
                }
            }
        }

        let parsed: GetNetworkInfoResult = debug_json_parse::<GetNetworkInfoResult>(
            raw,
            "PIVX getnetworkinfo (patched schema)",
        )?;
        Ok(parsed)
    }

    /// PIVX-safe version of getblockchaininfo that normalizes schema differences
    // -----------------------------------------------------------------
    pub(crate) fn get_blockchain_info_patched_core(
        rpc: &Client,
    ) -> anyhow::Result<bitcoincore_rpc::json::GetBlockchainInfoResult> {
        use serde_json::Value;
        use log::{info, debug};

        let chain = crate::chain_kind::chain_from_env();
        if chain != crate::chain_kind::ChainKind::Pivx {
            return Ok(rpc.get_blockchain_info()?);
        }

        debug!("(patched_core) Entered unified PIVX getblockchaininfo patcher");

        let mut raw_chain: Value = rpc.call("getblockchaininfo", &[])?;
        debug!("PIVX raw getblockchaininfo: {}", raw_chain);

        // --- normalize softforks (array → map) ---
        if let Some(softforks_val) = raw_chain.get_mut("softforks") {
            if softforks_val.is_array() {
                let arr = softforks_val.take();
                let mut map = serde_json::Map::new();
                if let Some(array) = arr.as_array() {
                    for sf in array {
                        if let Some(id) = sf.get("id").and_then(|v| v.as_str()) {
                            let mut fork = sf.clone();
                            if let Some(obj) = fork.as_object_mut() {
                                obj.entry("type").or_insert(Value::String("buried".into()));
                                obj.entry("active").or_insert(Value::Bool(true));
                            }
                            map.insert(id.to_string(), fork);
                        }
                    }
                }
                *softforks_val = Value::Object(map);
                info!("PIVX: patched getblockchaininfo.softforks array → map");
            }
        }

        // --- normalize bip9_softforks ---
        {
            let obj = raw_chain.as_object_mut().expect("PIVX blockchaininfo not object");
            match obj.get_mut("bip9_softforks") {
                Some(v) if v.is_array() => {
                    let arr = v.take();
                    let mut map = serde_json::Map::new();
                    if let Some(array) = arr.as_array() {
                        for sf in array {
                            if let Some(id) = sf.get("id").and_then(|v| v.as_str()) {
                                map.insert(id.to_string(), sf.clone());
                            }
                        }
                    }
                    obj.insert("bip9_softforks".into(), Value::Object(map));
                    info!("PIVX: patched bip9_softforks array → map");
                }
                Some(v) if v.is_object() => {}
                _ => {
                    obj.insert("bip9_softforks".into(), Value::Object(serde_json::Map::new()));
                    debug!("PIVX: injected bip9_softforks: {{}}");
                }
            }
        }

        // --- rename field for compatibility ---
        {
            let obj = raw_chain.as_object_mut().unwrap();
            if let Some(val) = obj.remove("initial_block_downloading") {
                obj.insert("initialblockdownload".to_string(), val);
                debug!("PIVX: renamed initial_block_downloading → initialblockdownload");
            }
        }

        // --- inject safe defaults ---
        let obj = raw_chain.as_object_mut().unwrap();
        if !obj.contains_key("mediantime") {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            obj.insert("mediantime".into(), Value::Number(now.into()));
        }

        let defaults: [(&str, Value); 10] = [
            ("pruned", Value::Bool(false)),
            ("chainwork", Value::String(String::new())),
            ("size_on_disk", Value::Number(0.into())),
            ("warnings", Value::String(String::new())),
            ("initialblockdownload", Value::Bool(false)),
            ("verificationprogress", Value::from(1.0_f64)),
            ("chain", Value::String("main".into())),
            ("upgrades", Value::Object(serde_json::Map::new())),
            ("pruneheight", Value::Number(0u64.into())),
            ("automatic_pruning", Value::Bool(false)),
        ];
        for (k, v) in defaults {
            obj.entry(k.to_string()).or_insert(v);
        }

        serde_json::from_value(raw_chain)
            .context("failed to parse PIVX getblockchaininfo after schema patch")
    }

    // Wrapper for `self`
    pub(crate) fn get_blockchain_info_patched(
        &self,
    ) -> anyhow::Result<bitcoincore_rpc::json::GetBlockchainInfoResult> {
        Self::get_blockchain_info_patched_core(&self.rpc)
    }

    fn header_from_rpc(
        rpc: &bitcoincore_rpc::Client,
        hash: bitcoin::BlockHash,
    ) -> bitcoin::block::Header {
        use bitcoin::pow::CompactTarget;
        use bitcoin::block::Version;
        use bitcoin::hashes::{sha256d, Hash};
        use log::warn;

        match rpc.call::<serde_json::Value>("getblockheader", &[serde_json::json!(hash.to_string())]) {
            Ok(raw) => {
                // ---- Parse fields safely ----
                let version_i32 = raw.get("version").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let version = Version::from_consensus(version_i32);

                // --- Make a reusable zero-hash placeholder ---
                let zero_hash =
                    sha256d::Hash::from_slice(&[0u8; 32]).expect("zero hash from slice must succeed");

                let prev_hash = raw
                    .get("previousblockhash")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<bitcoin::BlockHash>().ok())
                    .unwrap_or_else(|| bitcoin::BlockHash::from_raw_hash(zero_hash));

                let merkle_root = raw
                    .get("merkleroot")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<bitcoin::TxMerkleNode>().ok())
                    .unwrap_or_else(|| bitcoin::TxMerkleNode::from_raw_hash(zero_hash));

                let time = raw.get("time").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

                let bits = raw
                    .get("bits")
                    .and_then(|v| v.as_u64())
                    .map(|b| CompactTarget::from_consensus(b as u32))
                    .unwrap_or_else(|| CompactTarget::from_consensus(0));

                let nonce = raw.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

                bitcoin::block::Header {
                    version,
                    prev_blockhash: prev_hash,
                    merkle_root,
                    time,
                    bits,
                    nonce,
                }
            }
            Err(e) => {
                warn!("⚠️ failed to fetch header {} via getblockheader: {}", hash, e);

                let zero_hash =
                    sha256d::Hash::from_slice(&[0u8; 32]).expect("zero hash from slice must succeed");

                bitcoin::block::Header {
                    version: Version::from_consensus(0),
                    prev_blockhash: bitcoin::BlockHash::from_raw_hash(zero_hash),
                    merkle_root: bitcoin::TxMerkleNode::from_raw_hash(zero_hash),
                    time: 0,
                    bits: CompactTarget::from_consensus(0),
                    nonce: 0,
                }
            }
        }
    }

    // =====================================================
    // Override for get_block_header (PIVX-friendly)
    // =====================================================
    pub(crate) fn get_block_header(&self, hash: &bitcoin::BlockHash) -> anyhow::Result<bitcoin::block::Header> {
        use crate::chain_kind::{chain_from_env, ChainKind};

        let chain_kind = chain_from_env();
        if chain_kind == ChainKind::Pivx {
            return self.get_block_header_patched(hash);
        }

        Ok(self.rpc.get_block_header(hash)?)
    }

    /// PIVX-compatible header fetcher (converts JSON → minimal Bitcoin header)
    fn get_block_header_patched(&self, hash: &bitcoin::BlockHash) -> anyhow::Result<bitcoin::block::Header> {
        use log::debug;
        use serde_json::Value;
        use bitcoin::{block::Header, hashes::sha256d, TxMerkleNode};
        use bitcoin::hashes::Hash;
        use anyhow::Context;

        let raw: Value = self
            .rpc
            .call("getblockheader", &[hash.to_string().into(), false.into()])
            .context("RPC getblockheader failed")?;

        // Some pivxd versions return string (hex) if 'false' is passed; others return JSON
        if let Some(hex_str) = raw.as_str() {
            // Parse from hex (should be 160 characters → 80 bytes)
            let bytes = hex::decode(hex_str)?;
            let header: Header = bitcoin::consensus::encode::deserialize(&bytes)?;
            return Ok(header);
        }

        // Fallback: parse JSON fields manually
        let obj = raw.as_object().context("getblockheader: expected object")?;
        let version = obj.get("version").and_then(|v| v.as_i64()).unwrap_or_default() as i32;
        let prev_hash = obj.get("previousblockhash")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<bitcoin::BlockHash>().ok())
            .unwrap_or_else(|| bitcoin::BlockHash::all_zeros());
        let merkle_root = obj.get("merkleroot")
            .and_then(|v| v.as_str())
            .map(|s| TxMerkleNode::from_slice(&hex::decode(s).unwrap_or_default()).unwrap())
            .unwrap_or_else(|| TxMerkleNode::from_slice(&[0u8; 32]).unwrap());
        let time = obj.get("time").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let bits = obj.get("bits").and_then(|v| v.as_str()).unwrap_or("1d00ffff");
        let bits_u32 = u32::from_str_radix(bits, 16).unwrap_or(0);
        let nonce = obj.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

        let header = Header {
            version: bitcoin::block::Version::from_consensus(version),
            prev_blockhash: prev_hash,
            merkle_root,
            time,
            bits: bitcoin::CompactTarget::from_consensus(bits_u32),
            nonce,
        };

        debug!("✅ Parsed PIVX header {} @ {}", hash, time);
        Ok(header)
    }

    // =====================================================
    // Override for header synchronization
    // =====================================================
    pub(crate) fn get_new_headers(
        &self,
        chain: &crate::chain::Chain,
    ) -> anyhow::Result<Vec<crate::chain::NewHeader>> {
        use crate::chain_kind::{chain_from_env, ChainKind};

        let chain_kind = chain_from_env();

        if chain_kind == ChainKind::Pivx {
            return self.get_new_headers_patched(chain);
        }

        // ✅ default Bitcoin path (use P2P connection)
        Ok(self.p2p.lock().get_new_headers(chain)?)
    }

    // =====================================================
    // Override for for_blocks() — RPC-based fallback for PIVX
    // =====================================================
    pub(crate) fn for_blocks<B, F>(&self, blockhashes: B, mut func: F) -> anyhow::Result<()>
    where
        B: IntoIterator<Item = bitcoin::BlockHash>,
        F: FnMut(bitcoin::BlockHash, crate::types::SerBlock),
    {
        use crate::chain_kind::{chain_from_env, ChainKind};
        use bitcoin::consensus::encode::{deserialize, serialize};
        use log::{info, warn};
        use serde_json::Value;

        // ✅ Move this up here so both branches can use it
        let hashes: Vec<_> = blockhashes.into_iter().collect();

        if chain_from_env() == ChainKind::Pivx {
            info!("🔧 Using PIVX RPC for_blocks() (hash → raw block hex)");

            let total = hashes.len();
            for (i, hash) in hashes.iter().enumerate() {
                let params = vec![Value::String(hash.to_string()), Value::Bool(false)];
                match self.rpc.call::<String>("getblock", &params) {
                    Ok(hex_block) => {
                        match hex::decode(&hex_block)
                            .ok()
                            .and_then(|b| deserialize::<bitcoin::Block>(&b).ok())
                        {
                            Some(block) => {
                                let ser = serialize(&block); // Vec<u8>
                                func(*hash, ser);
                            }
                            None => warn!("⚠️ failed to decode block {}", hash),
                        }
                    }
                    Err(e) => {
                        warn!("⚠️ getblock(hash) failed for {}: {}", hash, e);
                    }
                }

                if i % 100 == 0 || i + 1 == total {
                    let pct = ((i + 1) as f64 / total as f64) * 100.0;
                    info!("📦 indexed {}/{} blocks ({:.2}%)", i + 1, total, pct);
                }
            }
            return Ok(());
        }

        // ✅ hashes is now in scope here
        Ok(self.p2p.lock().for_blocks(hashes, func)?)
    }

    // =====================================================
    // Helper: safely fetch and decode a single PIVX header
    // =====================================================
    fn fetch_pivx_header(
       rpc: &bitcoincore_rpc::Client,
       hash: &bitcoin::BlockHash,
       height: u64,
    ) -> anyhow::Result<bitcoin::block::Header> {
       use bitcoin::consensus::encode::deserialize;
       use serde_json::json;

       let hex_header = rpc.call::<String>("getblockheader", &[json!(hash), json!(false)])?;
       let bytes = hex::decode(&hex_header)?;
       let header: bitcoin::block::Header = deserialize(&bytes)?;

       // 🔧 Do NOT recompute and compare hash — PIVX hashes differ by design.
       Ok(header)
    }

    // =====================================================
    // PIVX header synchronization (RPC-only mode)
    // =====================================================
    pub(crate) fn get_new_headers_patched(
        &self,
        chain: &crate::chain::Chain,
    ) -> anyhow::Result<Vec<crate::chain::NewHeader>> {
        use log::{info, warn};
        use serde_json::json;
        use std::cmp::min;
        use std::time::Instant;
        use crate::chain::NewHeader;

        // -----------------------------------------------------
        // Configurable constants
        // -----------------------------------------------------
        let batch_size: u64 = std::env::var("PIVX_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(2000);

        let test_limit: Option<u64> = std::env::var("PIVX_HEADER_TEST_LIMIT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok());

        let skip_genesis = std::env::var("ELECTRS_SKIP_GENESIS")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);

        info!(
            "🔧 PIVX get_new_headers_patched(): full RPC header sync \
             (batch_size={}, skip_genesis={}, test_limit={:?})",
            batch_size, skip_genesis, test_limit
        );

        // -----------------------------------------------------
        // Step 1. Determine sync range
        // -----------------------------------------------------
        let tip_height: u64 = self.rpc.call("getblockcount", &[])?;
        let local_height = if skip_genesis { chain.height().max(1) as u64 } else { chain.height() as u64 };

        // -----------------------------------------------------
        // 🔒 Early exit if we've hit the test limit (for debugging)
        // -----------------------------------------------------
        if let Some(limit) = test_limit {
            if local_height >= limit {
                info!("🧩 Reached test limit ({}). Halting header sync.", limit);
                return Ok(vec![]);
            }
        }

        if tip_height <= local_height {
            info!("PIVX: already up to date (local={} remote={})", local_height, tip_height);
            return Ok(vec![]);
        }

        let target_height = test_limit.unwrap_or(tip_height);
        let max_height = min(tip_height, target_height);
        let total_headers = max_height - local_height;
        let start_time = Instant::now();

        info!(
            "📈 Starting PIVX header sync: {} → {} ({} headers)",
            local_height + 1,
            max_height,
            total_headers
        );

        // -----------------------------------------------------
        // Step 2. Fetch headers in batches
        // -----------------------------------------------------
        let mut headers = Vec::with_capacity(total_headers as usize);
        let mut next_height = local_height + 1;

        while next_height <= max_height {
            let batch_start = next_height;
            let batch_end = min(next_height + batch_size - 1, max_height);
            let pct = ((batch_end - local_height) as f64 / total_headers as f64) * 100.0;
            info!("⏳ Fetching headers {}–{} ({:.2}% complete)", batch_start, batch_end, pct);

            for h in batch_start..=batch_end {
                match self.rpc.get_block_hash(h) {
                    Ok(hash) => match Self::fetch_pivx_header(&self.rpc, &hash, h) {
                        Ok(header) => {
                            // ✅ Use canonical RPC hash (no placeholder)
                            let nh = crate::chain::NewHeader::with_canonical_hash(header, h as usize, hash);
                            headers.push(nh);
                        }
                        Err(e) => warn!("⚠️ header {} failed: {}", h, e),
                    },
                    Err(e) => warn!("⚠️ get_block_hash {} failed: {}", h, e),
                }
            }

            let elapsed_batch = start_time.elapsed().as_secs_f64();
            info!(
                "✅ Batch {}–{} done ({:.2}% total, {:.1}s elapsed)",
                batch_start, batch_end, pct, elapsed_batch
            );

            next_height = batch_end + 1;
        }


        info!(
            "🏁 Header sync complete — fetched {} headers ({}–{}) in {:.2}s",
            headers.len(),
            local_height + 1,
            max_height,
            start_time.elapsed().as_secs_f64()
        );

        Ok(headers)
    }



    pub(crate) fn connect(
        config: &Config,
        exit_flag: &ExitFlag,
        metrics: &Metrics,
    ) -> Result<Self> {
        let mut rpc = rpc_connect(config)?;

        // ================================================================
        // 🔧 Global PIVX schema preflight normalization
        // Ensures getnetworkinfo / getblockchaininfo are patched early
        // ================================================================
        let chain = chain_from_env();
        if chain == ChainKind::Pivx {
            info!("🔧 PIVX global preflight patch for getnetworkinfo/getblockchaininfo");

            // Normalize getnetworkinfo directly
            match Daemon::get_network_info_patched_with(&rpc) {
                Ok(_) => info!("PIVX: preflight getnetworkinfo normalized"),
                Err(e) => warn!("⚠️ PIVX preflight getnetworkinfo patch failed: {}", e),
            }

            // Quick sanity fetch for getblockchaininfo
            match rpc.call::<serde_json::Value>("getblockchaininfo", &[]) {
                Ok(raw) => debug!("PIVX preflight getblockchaininfo: {}", raw),
                Err(e) => warn!("⚠️ PIVX preflight getblockchaininfo failed: {}", e),
            }
        }

        // ---------------------------------------------------------------------------
        // Detect if we're running in PIVX mode and short-circuit Bitcoin P2P logic
        // ---------------------------------------------------------------------------
        let chain = chain_from_env();
        info!("PIVX debug: chain_from_env() = {:?}", chain);

        if chain == ChainKind::Pivx {
            info!("🔗 Detected PIVX chain — using pivxd RPC adapter");

            info!("PIVX RPC-only mode active — syncing via pivxd getblock RPC (no P2P threads)");

            // -------------------------------------------------------
            // Build the daemon once
            // -------------------------------------------------------
            let daemon = Daemon {
                p2p: Mutex::new(Connection::connect(
                    config.network,
                    config.daemon_p2p_addr,
                    metrics,
                    config.signet_magic,
                    &config,
                )?),
                rpc,
            };

            // -------------------------------------------------------
            // Schema normalization & validation
            // -------------------------------------------------------
            let _net = daemon
                .get_network_info_patched()
                .context("failed to parse PIVX getnetworkinfo after schema patch")?;

            let _chain_info = daemon
                .get_blockchain_info_patched()
                .context("failed to parse PIVX getblockchaininfo after schema patch")?;

            info!("✅ PIVX: validated schema for both getnetworkinfo and getblockchaininfo");

            // -------------------------------------------------------
            // Ready to return
            // -------------------------------------------------------
            return Ok(daemon);
        }


        loop {
            exit_flag
                .poll()
                .context("bitcoin RPC polling interrupted")?;
            match rpc_poll(&mut rpc, config.skip_block_download_wait) {
                PollResult::Done(result) => {
                    result.context("bitcoind RPC polling failed")?;
                    break; // on success, finish polling
                }
                PollResult::Retry => {
                    std::thread::sleep(std::time::Duration::from_secs(1)); // wait a bit before polling
                }
            }
        }

        debug!("⚠️ Raw get_network_info() called let network_info_raw: serde_json::Value after loop");

        let network_info_raw: serde_json::Value = rpc.call("getnetworkinfo", &[])?;
        debug!("🔍 Raw getnetworkinfo before parse: {}", network_info_raw);
        let network_info: bitcoincore_rpc::json::GetNetworkInfoResult =
            debug_json_parse::<bitcoincore_rpc::json::GetNetworkInfoResult>(
                network_info_raw,
                "final getnetworkinfo parse",
            )?;
        let info = rpc.get_blockchain_info()?;

        if chain == ChainKind::Bitcoin && network_info.version < 21_00_00 {
            bail!("electrs requires bitcoind 0.21+");
        }

        if chain == ChainKind::Bitcoin && !network_info.network_active {
            bail!("electrs requires active bitcoind p2p network");
        }

        // For Bitcoin nodes, ensure they aren't pruned
        if chain == ChainKind::Bitcoin && info.pruned {
            bail!("electrs requires non-pruned bitcoind node");
        }

        let p2p = Mutex::new(Connection::connect(
            config.network,
            config.daemon_p2p_addr,
            metrics,
            config.signet_magic,
            &config,
        )?);

        Ok(Self { p2p, rpc })
    }

    pub(crate) fn estimate_fee(&self, nblocks: u16) -> Result<Option<Amount>> {
        let res = self.rpc.estimate_smart_fee(nblocks, None);
        if let Err(bitcoincore_rpc::Error::JsonRpc(jsonrpc::Error::Rpc(RpcError {
            code: -32603,
            ..
        }))) = res
        {
            return Ok(None); // don't fail when fee estimation is disabled (e.g. with `-blocksonly=1`)
        }
        Ok(res.context("failed to estimate fee")?.fee_rate)
    }

    pub(crate) fn get_relay_fee(&self) -> Result<Amount> {
        Ok(self
            .get_network_info_patched()
            .context("failed to get relay fee (PIVX-patched)")?
            .relay_fee)
    }

    pub(crate) fn broadcast(&self, tx: &Transaction) -> Result<Txid> {
        self.rpc
            .send_raw_transaction(tx)
            .context("failed to broadcast transaction")
    }

    pub(crate) fn get_transaction_info(
        &self,
        txid: &Txid,
        blockhash: Option<BlockHash>,
    ) -> Result<Value> {
        // No need to parse the resulting JSON, just return it as-is to the client.
        self.rpc
            .call(
                "getrawtransaction",
                &[json!(txid), json!(true), json!(blockhash)],
            )
            .context("failed to get transaction info")
    }

    pub(crate) fn get_transaction_hex(
        &self,
        txid: &Txid,
        blockhash: Option<BlockHash>,
    ) -> Result<Value> {
        use bitcoin::consensus::serde::{hex::Lower, Hex, With};

        let tx = self.get_transaction(txid, blockhash)?;
        #[derive(serde::Serialize)]
        #[serde(transparent)]
        struct TxAsHex(#[serde(with = "With::<Hex<Lower>>")] Transaction);
        serde_json::to_value(TxAsHex(tx)).map_err(Into::into)
    }

    pub(crate) fn get_transaction(
        &self,
        txid: &Txid,
        blockhash: Option<BlockHash>,
    ) -> Result<Transaction> {
        self.rpc
            .get_raw_transaction(txid, blockhash.as_ref())
            .context("failed to get transaction")
    }

    pub(crate) fn get_block_txids(&self, blockhash: BlockHash) -> Result<Vec<Txid>> {
        Ok(self
            .rpc
            .get_block_info(&blockhash)
            .context("failed to get block txids")?
            .tx)
    }

    pub(crate) fn get_mempool_info(&self) -> Result<json::GetMempoolInfoResult> {
        // --- Detect if we're running in PIVX mode ---
        let is_pivx = std::env::var("ELECTRS_CHAIN")
            .map(|v| v.to_lowercase() == "pivx")
            .unwrap_or(false);

        if is_pivx {
            use bitcoin::Amount;
            use bitcoincore_rpc::jsonrpc;
            use serde_json::value::RawValue;

            // Fetch mempool txids
            let txids: Vec<String> = self
                .rpc
                .call("getrawmempool", &[json!(false)])
                .context("failed to get raw mempool")?;

            let size = txids.len();
            let bytes = size * 250;
            let usage = bytes;
            let max_mempool = 300_000_000usize;
            let mempool_min_fee = Amount::from_sat(1000);
            let min_relay_tx_fee = Amount::from_sat(1000);
            let incremental_relay_fee = Some(Amount::from_sat(1000));

            // --- Compute total_fee via batch getmempoolentry ---
            let mut total_fee_sat: u64 = 0;
            if !txids.is_empty() {
                let client = self.rpc.get_jsonrpc_client();

                // Build list of RawValue arguments
                let args: Vec<Box<RawValue>> = txids
                    .iter()
                    .map(|txid| {
                        jsonrpc::try_arg([txid])
                            .context("failed to serialize txid into JSON")
                            .unwrap()
                    })
                    .collect();

                let reqs: Vec<jsonrpc::Request> = args
                    .iter()
                    .map(|arg| client.build_request("getmempoolentry", Some(arg)))
                    .collect();

                match client.send_batch(&reqs) {
                    Ok(results) => {
                        for resp_opt in results {
                            if let Some(resp) = resp_opt {
                                // Each Response -> attempt to parse as serde_json::Value
                                if let Ok(entry) = resp.result::<serde_json::Value>() {
                                    if let Some(fee_val) = entry
                                        .get("fee")
                                        .and_then(|f: &serde_json::Value| f.as_f64())
                                    {
                                        total_fee_sat =
                                            total_fee_sat.saturating_add((fee_val * 1e8) as u64);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("PIVX: batch getmempoolentry failed: {}", e);
                    }
                }
            }

            let pivx_info = json::GetMempoolInfoResult {
                size,
                bytes,
                usage,
                max_mempool,
                mempool_min_fee,
                min_relay_tx_fee,
                incremental_relay_fee,
                full_rbf: Some(false),
                loaded: Some(true),
                unbroadcast_count: Some(0),
                total_fee: Some(Amount::from_sat(total_fee_sat)),
            };

            log::info!(
                "PIVX mempool info synthesized: {} txs (~{} bytes, total_fee={} PIV)",
                size,
                bytes,
                total_fee_sat as f64 / 1e8
            );

            return Ok(pivx_info);
        }

        // --- Default Bitcoin path ---
        self.rpc
            .get_mempool_info()
            .context("failed to get mempool info")
    }


    pub(crate) fn get_mempool_txids(&self) -> Result<Vec<Txid>> {
        self.rpc
            .get_raw_mempool()
            .context("failed to get mempool txids")
    }

    pub(crate) fn get_mempool_entries(
        &self,
        txids: &[Txid],
    ) -> Result<Vec<Option<json::GetMempoolEntryResult>>> {
        let results = batch_request(self.rpc.get_jsonrpc_client(), "getmempoolentry", txids)?;
        Ok(results
            .into_iter()
            .map(|r| match r?.result::<json::GetMempoolEntryResult>() {
                Ok(entry) => Some(entry),
                Err(err) => {
                    debug!("failed to get mempool entry: {}", err); // probably due to RBF
                    None
                }
            })
            .collect())
    }

    pub(crate) fn get_mempool_transactions(
        &self,
        txids: &[Txid],
    ) -> Result<Vec<Option<Transaction>>> {
        let results = batch_request(self.rpc.get_jsonrpc_client(), "getrawtransaction", txids)?;
        Ok(results
            .into_iter()
            .map(|r| -> Option<Transaction> {
                let tx_hex = match r?.result::<String>() {
                    Ok(tx_hex) => Some(tx_hex),
                    Err(err) => {
                        debug!("failed to get mempool tx: {}", err); // probably due to RBF
                        None
                    }
                }?;
                let tx_bytes = match Vec::from_hex(&tx_hex) {
                    Ok(tx_bytes) => Some(tx_bytes),
                    Err(err) => {
                        warn!("got non-hex transaction {}: {}", tx_hex, err);
                        None
                    }
                }?;
                match deserialize(&tx_bytes) {
                    Ok(tx) => Some(tx),
                    Err(err) => {
                        warn!("got invalid tx {}: {}", tx_hex, err);
                        None
                    }
                }
            })
            .collect())
    }

    //pub(crate) fn get_new_headers(&self, chain: &Chain) -> Result<Vec<NewHeader>> {
    //    self.p2p.lock().get_new_headers(chain)
    //}

    //pub(crate) fn for_blocks<B, F>(&self, blockhashes: B, func: F) -> Result<()>
    //where
    //    B: IntoIterator<Item = BlockHash>,
    //    F: FnMut(BlockHash, SerBlock),
    //{
    //    self.p2p.lock().for_blocks(blockhashes, func)
    //}

    pub(crate) fn new_block_notification(&self) -> Receiver<()> {
        self.p2p.lock().new_block_notification()
    }
}

pub(crate) type RpcError = bitcoincore_rpc::jsonrpc::error::RpcError;

pub(crate) fn extract_bitcoind_error(err: &bitcoincore_rpc::Error) -> Option<&RpcError> {
    use bitcoincore_rpc::{
        jsonrpc::error::Error::Rpc as ServerError, Error::JsonRpc as JsonRpcError,
    };
    match err {
        JsonRpcError(ServerError(e)) => Some(e),
        _ => None,
    }
}

fn batch_request<T>(
    client: &jsonrpc::Client,
    name: &str,
    items: &[T],
) -> Result<Vec<Option<jsonrpc::Response>>>
where
    T: Serialize,
{
    debug!("calling {} on {} items", name, items.len());
    let args: Vec<Box<RawValue>> = items
        .iter()
        .map(|item| jsonrpc::try_arg([item]).context("failed to serialize into JSON"))
        .collect::<Result<Vec<_>>>()?;
    let reqs: Vec<jsonrpc::Request> = args
        .iter()
        .map(|arg| client.build_request(name, Some(arg)))
        .collect();
    match client.send_batch(&reqs) {
        Ok(values) => {
            assert_eq!(items.len(), values.len());
            Ok(values)
        }
        Err(err) => bail!("batch {} request failed: {}", name, err),
    }
}
