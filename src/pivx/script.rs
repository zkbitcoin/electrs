// src/pivx/script.rs
// -----------------------------------------------------
// Generic PIVX script classification (Phase-1)
// No Zerocoin, Coldstake, Sapling logic yet.
// All PIVX-only types return as Unknown.
// -----------------------------------------------------

use bitcoin::blockdata::script::Instruction;
use bitcoin::blockdata::script::Script;

/// Basic script class for PIVX (Phase-1)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptClass {
    P2PKH,
    P2SH,
    P2PK,
    NullData,
    NonStandard,
    Unknown,
}

/// Main dispatch
pub fn classify_script(script: &Script) -> ScriptClass {
    if script.is_provably_unspendable() {
        return ScriptClass::NullData;
    }

    // Match: OP_DUP OP_HASH160 <20-byte> OP_EQUALVERIFY OP_CHECKSIG
    if script.is_p2pkh() {
        return ScriptClass::P2PKH;
    }

    // Match: OP_HASH160 <20-byte> OP_EQUAL
    if script.is_p2sh() {
        return ScriptClass::P2SH;
    }

    // P2PK: <pubkey> OP_CHECKSIG
    if is_p2pk(script) {
        return ScriptClass::P2PK;
    }

    // Others — Phase-2 will handle:
    //  • Coldstake P2CS
    //  • Cold/Hot stake split scripts
    //  • Zerocoin spends/mints
    //  • Sapling shielded anchors
    ScriptClass::Unknown
}

/// Minimal P2PK recognizer
fn is_p2pk(script: &Script) -> bool {
    // <33 or 65 byte pubkey> OP_CHECKSIG
    let mut iter = script.instructions();
    let pk = match iter.next() {
        Some(Ok(Instruction::PushBytes(b))) => b,
        _ => return false,
    };
    match iter.next() {
        Some(Ok(Instruction::Op(op))) if op.to_u8() == bitcoin::blockdata::opcodes::all::OP_CHECKSIG.to_u8() => {
            // valid pubkey lengths: 33 (compressed) or 65 (uncompressed)
            pk.len() == 33 || pk.len() == 65
        }
        _ => false,
    }
}
