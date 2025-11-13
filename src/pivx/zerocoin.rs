use crate::pivx::types::{ZerocoinData, ZerocoinType};

pub fn detect_zerocoin_outputs() -> ZerocoinData {
    ZerocoinData {
        has_zerocoin: false,
        zc_type: ZerocoinType::Unknown,
    }
}
