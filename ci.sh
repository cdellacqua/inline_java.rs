#!/bin/sh
set -eu

cargo fmt --all
cargo clippy --fix --allow-dirty --workspace --all-targets -- -D warnings -D clippy::pedantic
cargo test --workspace --locked
