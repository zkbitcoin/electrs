#!/bin/bash

export LIBCLANG_PATH="/opt/homebrew/opt/llvm/lib"
export LD_LIBRARY_PATH="/opt/homebrew/opt/llvm/lib"
export DYLD_LIBRARY_PATH="/opt/homebrew/opt/llvm/lib"
export CPATH="/opt/homebrew/include"
export LIBRARY_PATH="/opt/homebrew/lib"

export PATH="/opt/homebrew/opt/llvm/bin:$PATH"

cargo clean
cargo build --release
