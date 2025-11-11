#!/bin/bash

ELECTRS_CHAIN=pivx ./target/release/electrs --daemon-rpc-addr 127.0.0.1:<pivx-port> --auth user:pass --db-dir ./electrs_db_pivx

