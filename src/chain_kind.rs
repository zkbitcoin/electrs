// src/chain_kind.rs
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ChainKind { Bitcoin, Pivx }

pub fn chain_from_env() -> ChainKind {
    match std::env::var("ELECTRS_CHAIN")
        .unwrap_or_else(|_| "bitcoin".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "pivx" => ChainKind::Pivx,
        _      => ChainKind::Bitcoin,
    }
}
