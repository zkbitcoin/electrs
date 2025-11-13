Suggested Timeline
Phase 1 — Full PIVX Block Parser (YOU listed A–I)

transparent inputs/outputs

cold/hot stake

coinstake structure

zerocoin mint/spend v1–v3

sapling spends/outputs

shielded value balances

binding signature

block reward extraction (raw, unfixed)

Goal:
Electrs-PIVX can sync entire chain without crashing, mis-parsing, or skipping anything.

Phase 2 — PIVX API (Blockbook-compatible)

/api/v2/tx/…

/api/v2/block/…

/api/v2/address/…

/api/v2/utxo/…

/api/v2/balance/…

Blockbook parsing sucks for PIVX (cold/hot stake broken).
We fix that.

Phase 3 — Reward Engine

manual reward schedule

staking reward classification

masternode reward classification

treasury payout classification

coinstake edge graph (your analyzer logic)

Phase 4 — Masternode Module

collateral detection

MN history

operator clusters

payout stats

missed blocks

Phase 5 — Treasury Flow / Governance

superblock inference

payouts

proposal analytics

N-hop flow tracking

address type classification
