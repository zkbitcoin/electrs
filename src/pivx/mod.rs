// src/pivx/mod.rs
pub mod block;
pub mod tx;
pub mod script;
pub mod coinstake;
pub mod coldstake;
pub mod zerocoin;
pub mod sapling;
pub mod types;

use bitcoin::Network;

/// Whether chain_name == "pivx"
pub fn is_pivx_chain(chain_name: &str) -> bool {
    chain_name.eq_ignore_ascii_case("pivx")
}

/// Return PIVX mainnet Sapling activation height (hardcoded for now).
pub fn sapling_activation_height() -> u32 {
    2150000   // PIVX v5.0 activation height (Sapling)
}

/// Whether a block height is post-Sapling.
pub fn is_sapling_height(height: u32) -> bool {
    height >= sapling_activation_height()
}
