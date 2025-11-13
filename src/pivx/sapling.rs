use crate::pivx::types::{SaplingData, SaplingSpend, SaplingOutput};

pub fn detect_sapling() -> SaplingData {
    SaplingData {
        spends: vec![],
        outputs: vec![],
        value_balance: 0,
        has_binding_sig: false,
    }
}
