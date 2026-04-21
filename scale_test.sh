#!/bin/bash
set -euo pipefail

echo "Running sequential cold-start benchmark via bench_local..."
cargo run --bin bench_local --release -- "$@"
