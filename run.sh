#!/bin/bash

RUST_LOG=info RUST_BACKTRACE=1 ELECTRS_CHAIN=pivx PIVX_HEADER_TEST_LIMIT=10 ./target/release/electrs --conf electrs_pivx.toml
