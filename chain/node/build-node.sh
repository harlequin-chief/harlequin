#!/bin/bash
# Build wrapper for the Harlequin node. Exports the env native build deps need so it reaches cargo's
# build scripts (background/nohup launches strip them):
#   - LIBCLANG_PATH / LLVM_CONFIG_PATH: clang-sys/bindgen (rocksdb).   apt: libclang-dev libclang1-19 llvm-dev
#   - PROTOC: prost-build/litep2p (protobuf compiler).                  apt: protobuf-compiler
# Paths below are Debian/LLVM-19 defaults; adjust for your distro.
export LIBCLANG_PATH="${LIBCLANG_PATH:-/usr/lib/x86_64-linux-gnu}"
export LLVM_CONFIG_PATH="${LLVM_CONFIG_PATH:-/usr/bin/llvm-config}"
export PROTOC="${PROTOC:-/usr/bin/protoc}"
cd "$(dirname "$0")" || exit 1
exec cargo build --release
