use serde::{Serialize, Deserialize};

/// Zerocoin type classification (stub)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ZerocoinType {
    MintV1,
    MintV2,
    SpendV3,
    Unknown,
}

/// Parsed Zerocoin result (stub)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZerocoinData {
    pub has_zerocoin: bool,
    pub zc_type: ZerocoinType,
}

/// Sapling spend/output (stub structures)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaplingSpend {
    pub dummy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaplingOutput {
    pub dummy: bool,
}

/// Sapling parsed data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaplingData {
    pub spends: Vec<SaplingSpend>,
    pub outputs: Vec<SaplingOutput>,
    pub value_balance: i64,
    pub has_binding_sig: bool,
}

/// Parsed PIVX output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PivxTxOut {
    pub value: u64,
    pub script_hex: String,
    pub is_coldstake: bool,
}

/// FINAL parsed PIVX tx result (what tx.rs builds)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PivxParsedTx {
    pub is_coinstake: bool,
    pub zerocoin: ZerocoinData,
    pub sapling: SaplingData,
    pub outputs: Vec<PivxTxOut>,
}
