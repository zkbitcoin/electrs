#!/bin/bash

RUST_LOG=trace,electrs::index=debug,electrs::chain=debug,electrs::db=debug RUST_BACKTRACE=1 ./target/release/electrs --conf electrs_pivx.toml
