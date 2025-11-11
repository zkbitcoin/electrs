// src/pivx/mod.rs
// =============================================================================
// PIVX chain helpers for electrs-pivx
//   - network params
//   - coinstake / reward classification
//   - cold & hot staking detection
//   - optional shielded TX detection
// =============================================================================

use sha2::{Digest, Sha256};
use hex;
use bitcoin::{Transaction, Txid};
use bitcoin::hashes::{sha256d, Hash};
use bitcoin_slices::{bsl, Visitor, Visit};
use serde_json::Value;
use std::ops::ControlFlow;

// =============================================================================
// Base58 / script helpers
// =============================================================================

pub struct PivxParams {
    pub p2pkh_ver: u8,
    pub p2sh_ver:  u8,
    pub wif_ver:   u8,
    pub genesis_hash_hex: &'static str,
}

impl PivxParams {
    pub const fn mainnet() -> Self {
        Self {
            p2pkh_ver: 30,
            p2sh_ver:  13,
            wif_ver:   212,
            genesis_hash_hex:
                "00000d5dbb9bc758a9d2ebbd3b3c0f7ca2d5989a5e96d64f6b1a56f2c1f6a33c",
        }
    }
}

pub fn scripthash_hex(script: &[u8]) -> String {
    let mut h = Sha256::digest(script);
    h = Sha256::digest(&h);
    let mut v = h.to_vec();
    v.reverse();
    hex::encode(v)
}

// =============================================================================
// Transaction helpers
// =============================================================================

/// Detect classic PIVX coinstake (bitcoin::Transaction)
pub fn detect_pivx_coinstake(tx: &Transaction) -> bool {
    if tx.input.is_empty() || tx.output.len() < 2 {
        return false;
    }
    // skip coinbase (null prevout)
    let zero_txid = Txid::from_raw_hash(sha256d::Hash::all_zeros());
    if tx.input[0].previous_output.txid == zero_txid {
        return false;
    }
    tx.output[0].value.to_sat() == 0
}

/// Detect PIVX-style coinstake transactions using a `bsl::Transaction`
/// - Skips coinbase (null prevout)
/// - Returns true if the first output has 0 value
pub fn detect_pivx_coinstake_bsl(tx: &bsl::Transaction) -> bool {
    // First seen prev_txid and output value
    let mut first_prev_txid: Option<[u8; 32]> = None;
    let mut first_value: Option<u64> = None;

    // Visitor to collect first vin/vout info
    struct FirstIOVisitor<'a> {
        first_prev_txid: &'a mut Option<[u8; 32]>,
        first_value: &'a mut Option<u64>,
    }

    impl<'a> Visitor for FirstIOVisitor<'a> {
        fn visit_tx_in(&mut self, _idx: usize, txin: &bsl::TxIn) -> ControlFlow<()> {
            if self.first_prev_txid.is_none() {
                // txid() returns &[u8]; copy into [u8; 32]
                let mut id = [0u8; 32];
                id.copy_from_slice(txin.prevout().txid());
                *self.first_prev_txid = Some(id);
            }
            ControlFlow::Continue(())
        }

        fn visit_tx_out(&mut self, _idx: usize, txout: &bsl::TxOut) -> ControlFlow<()> {
            if self.first_value.is_none() {
                *self.first_value = Some(txout.value());
            }
            ControlFlow::Continue(())
        }
    }

    // Walk transaction — pass the raw bytes to `visit`
    let mut v = FirstIOVisitor {
        first_prev_txid: &mut first_prev_txid,
        first_value: &mut first_value,
    };
    let _ = bsl::Transaction::visit(tx.as_ref(), &mut v);

    // Decide if this is coinstake
    match (first_prev_txid, first_value) {
        (Some(prev_txid), Some(val)) => {
            if prev_txid == [0u8; 32] {
                false // coinbase
            } else {
                val == 0 // coinstake = first output is 0
            }
        }
        _ => false,
    }
}

/// Height-aware reward classification
pub fn classify_pivx_output(
    value: f64,
    _vout_index: usize,
    total_vouts: usize,
    height: usize,
) -> &'static str {
    let (stake_val, mn_val, min_treasury) = if height < 900_000 {
        (2.5, 2.5, 500.0)
    } else {
        (4.0, 6.0, 1000.0)
    };
    if total_vouts > 3 && value > min_treasury {
        "treasury_reward"
    } else if (value - mn_val).abs() < 0.05 {
        "masternode_reward"
    } else if (value - stake_val).abs() < 0.05 {
        "staking_reward"
    } else {
        "regular"
    }
}

/// Detect whether a scriptPubKey uses OP_CHECKCOLDSTAKEVERIFY (0xb8)
pub fn is_coldstake_script(script: &[u8]) -> bool {
    script.contains(&0xb8)
}

/// Distinguish cold vs hot stake
pub fn classify_cold_hot_stake(script: &[u8]) -> &'static str {
    if !is_coldstake_script(script) {
        return "non_stake";
    }
    let hash160s = script.iter().filter(|&&b| b == 0xa9).count();
    match hash160s {
        1 => "cold_stake",
        2 => "hot_stake",
        _ => "unknown_stake",
    }
}

// =============================================================================
// Shielded TX (optional, RPC JSON only)
// =============================================================================

pub fn is_zerocoin_tx(json_tx: &Value) -> bool {
    json_tx.get("zerocoinmint").is_some() || json_tx.get("zerocoinspend").is_some()
}

pub fn is_sapling_tx(json_tx: &Value) -> bool {
    json_tx.get("valueBalance").is_some()
        || json_tx.get("vShieldedOutput").is_some()
        || json_tx.get("vShieldedSpend").is_some()
}

pub fn is_shielded_tx(json_tx: &Value) -> bool {
    is_zerocoin_tx(json_tx) || is_sapling_tx(json_tx)
}
