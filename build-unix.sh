#!/bin/bash

export CXXFLAGS="-std=c++17"
export CFLAGS="-std=c17"
export ROCKSDB_INCLUDE_DIR=/usr/include
export ROCKSDB_LIB_DIR=/usr/lib

cargo clean
cargo build --release
