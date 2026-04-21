#!/usr/bin/env bash
set -euo pipefail

cargo run --bin bench_size --release -- "$@"
