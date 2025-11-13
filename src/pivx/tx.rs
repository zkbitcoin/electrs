use bitcoin::{Transaction, TxOut};

use crate::pivx::{
    script::classify_script,
    coinstake::{is_coinstake, is_coldstake_vout},
    zerocoin::detect_zerocoin_outputs,
    sapling::detect_sapling,
    types::{PivxParsedTx, PivxTxOut, ZerocoinData, SaplingData},
};

/// Phase-1 stub: parse a PIVX transaction
pub fn parse_pivx_tx(tx: &Transaction, _height: u32) -> PivxParsedTx {
    let is_cs = is_coinstake(tx);

    let zerocoin: ZerocoinData = detect_zerocoin_outputs();

    let sapling: SaplingData = detect_sapling();

    let mut outputs = vec![];
    for vout in &tx.output {
        let is_cold = is_coldstake_vout(vout);

        outputs.push(PivxTxOut {
            value: vout.value.to_sat(),
            script_hex: hex::encode(vout.script_pubkey.as_bytes()),
            is_coldstake: is_cold,
        });
    }

    PivxParsedTx {
        is_coinstake: is_cs,
        zerocoin,
        sapling,
        outputs,
    }
}
