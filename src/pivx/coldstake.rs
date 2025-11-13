use bitcoin::{Transaction, TxOut, Script};

/// Detect PIVX coinstake (stub)
pub fn is_coinstake(tx: &Transaction) -> bool {
    tx.output.len() >= 2 && tx.output[0].value == bitcoin::Amount::from_sat(0)
}

/// Detect cold-stake vout (stub)
pub fn is_coldstake_vout(vout: &TxOut) -> bool {
    // Placeholder — will detect real P2CS later
    vout.script_pubkey.is_op_return()
}
