#!/bin/sh
set -eu

cargo fmt --all
cargo clippy --fix --allow-dirty --workspace --all-targets -- -D warnings -D clippy::pedantic
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test --workspace --locked
