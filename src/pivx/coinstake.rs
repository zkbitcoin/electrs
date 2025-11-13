use bitcoin::{Transaction, TxOut};

/// Detect PIVX coinstake transaction (Phase-1 stub)
pub fn is_coinstake(tx: &Transaction) -> bool {
    // In PIVX: coinstake vout[0].value == 0, stake rewards >= 2 vouts
    tx.output.len() >= 2
        && tx.output[0].value.to_sat() == 0
}

/// Detect cold-stake vout (Phase-1 stub)
///
/// IMPORTANT: signature takes &TxOut (NOT usize)
pub fn is_coldstake_vout(vout: &TxOut) -> bool {
    // Phase-1 placeholder: will detect real P2CS scripts later
    // For now: treat OP_RETURN as stub cold-stake indicator
    vout.script_pubkey.is_op_return()
}
