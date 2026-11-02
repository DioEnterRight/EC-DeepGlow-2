#!/bin/sh
# Rebuild manifest.h from manifest.json is done by gen_manifest.py (kept next to this file); then:
set -e
clang --target=wasm32 -O3 -ffreestanding -fno-builtin -fno-math-errno -nostdlib -Wall -Wextra -Wno-unused-parameter \
  -Wl,--no-entry -Wl,--allow-undefined -Wl,-z,stack-size=16384 -Wl,--initial-memory=65536 -Wl,--global-base=1024 \
  -Wl,--strip-all deep_glow_open.c -o ec_deep_glow_2.wasm
ls -l ec_deep_glow_2.wasm
