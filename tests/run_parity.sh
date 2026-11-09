#!/bin/sh
# Atajo: regenera los fixtures y ejecuta la paridad del core en Rust.  sh tests/run_parity.sh
set -e
sh tests/regen_golden.sh
(cd core && cargo test --release --test golden)
