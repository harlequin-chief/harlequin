#!/bin/bash
# Build wrapper for the Harlequin node. Exports the env native build deps need so it reaches cargo's
# build scripts (background/nohup launches strip them):
#   - LIBCLANG_PATH / LLVM_CONFIG_PATH: clang-sys/bindgen (rocksdb).   apt: libclang-dev libclang1-19 llvm-dev
#   - PROTOC: prost-build/litep2p (protobuf compiler).                  apt: protobuf-compiler
# Paths below are Debian/LLVM-19 defaults; adjust for your distro.
#
# ⚠️  MAINNET vs TESTNET (root cause of the 2026-06-30 finality halt — DO NOT repeat):
#   The `mainnet` cargo feature selects PRODUCTION consensus/reputation cadence (epoch 600, decay
#   half-life ~2y, beta=12, tau=60). WITHOUT it the build uses fast TESTNET values → on a real launch
#   the founder committee decays to 0 in ~5 epochs → finality halts. PRODUCTION builds MUST pass
#   `--mainnet`. The default stays testnet (fast validation), but prints a loud warning so a testnet
#   binary is never shipped to production by accident.
#     ./build-node.sh            → TESTNET build (dev/validation)  [warns]
#     ./build-node.sh --mainnet  → MAINNET build (production)
set -uo pipefail
export LIBCLANG_PATH="${LIBCLANG_PATH:-/usr/lib/x86_64-linux-gnu}"
export LLVM_CONFIG_PATH="${LLVM_CONFIG_PATH:-/usr/bin/llvm-config}"
export PROTOC="${PROTOC:-/usr/bin/protoc}"
cd "$(dirname "$0")" || exit 1

FEATURES=()
MODE="TESTNET"
for arg in "$@"; do
  case "$arg" in
    --mainnet) FEATURES=(--features mainnet); MODE="MAINNET";;
    --testnet) FEATURES=(); MODE="TESTNET";;
    *) echo "build-node.sh: unknown arg '$arg' (use --mainnet | --testnet)" >&2; exit 2;;
  esac
done


# --- RUTAS REMAPEADAS (2026-09-07) ---------------------------------------------------------------
# POR QUÉ. El binario publicado llevaba dentro la ruta de compilación: `/home/<cuenta>/…` aparecía
# 3.550 veces en el x86 de /dist y 3.896 en el ARM. Dos consecuencias, y las dos malas:
#   1. FUGA. En un proyecto cuyo argumento es participar sin dar identidad, repartíamos el nombre de
#      la cuenta de la máquina que compila. No es un nombre real, pero es un identificador estable,
#      que es justo lo que sirve para atar unas cosas con otras.
#   2. IRREPRODUCIBILIDAD. El sha solo salía igual compilando en la MISMA ruta absoluta — medido el
#      07-sep: misma marca en otra carpeta da otro sha; misma marca en la misma carpeta da el mismo,
#      bit a bit. O sea que el número que publicábamos para que cualquiera nos verificase NO lo podía
#      reproducir nadie de fuera, y su única conclusión posible era que mentíamos.
# Las dos se arreglan con lo mismo. OJO: hay que remapear LAS TRES familias — la mayor parte de las
# apariciones (3.457 de 3.550) venía del registro de cargo, NO de nuestro código, así que mover el
# proyecto de carpeta no habría servido de nada.
# Los destinos son fijos y neutros A PROPÓSITO: quien quiera verificarnos debe usar estos mismos.
export RUSTFLAGS="${RUSTFLAGS:-} \
  --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/hlq-build/cargo \
  --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/hlq-build/rustup \
  --remap-path-prefix=$(cd .. && pwd)=/hlq-build/harlequin"
# ...Y EL RUNTIME WASM TAMBIÉN (2026-09-17). substrate-wasm-builder compila el runtime en OTRA invocación
# de cargo y le PISA RUSTFLAGS con los suyos (wasm_project.rs: `.env("RUSTFLAGS", rustflags)`), así que el
# remapeo de arriba nunca le llegaba. Medido: el runtime spec 3 que está EN LA CADENA VIVA lleva 96 rutas
# `/home/<cuenta>/…` (cargo registry, rustup y la carpeta del proyecto) y ninguna `/hlq-build/`. Ese era
# el «algo del entorno» que hacía distintos A y B, y la misma fuga que este bloque dice cerrar. La única
# puerta que el builder deja abierta es WASM_BUILD_RUSTFLAGS. Comprobar con ops/hlq-build-paths.py.
export WASM_BUILD_RUSTFLAGS="${WASM_BUILD_RUSTFLAGS:-} --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/hlq-build/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/hlq-build/rustup --remap-path-prefix=$(cd .. && pwd)=/hlq-build/harlequin"


if [ "$MODE" = "MAINNET" ]; then
  echo ">>> Building MAINNET binary (--features mainnet): PRODUCTION cadence." >&2
else
  echo ">>> Building TESTNET binary (fast dev values). For PRODUCTION pass --mainnet." >&2
  echo ">>> ⚠️  A testnet binary launched on mainnet WILL halt finality (founder decay). Do NOT ship it." >&2
fi
cargo build --release "${FEATURES[@]}" || exit $?
BIN="${CARGO_TARGET_DIR:-target}/release/harlequin-node"

# CANDADO (2026-09-17): el build FALLA si el binario —o el runtime wasm comprimido que lleva dentro— conserva
# rutas de la máquina. Avisar no basta: el remapeo existía desde el 07-sep y el wasm lo perdía en silencio,
# porque nadie miraba dentro. Busca el candado junto al proyecto; si no lo encuentra, falla igual.
GUARD_PY="${HLQ_BUILD_PATHS_GUARD:-$(cd .. && cd .. && pwd)/ops/hlq-build-paths.py}"
[ -f "$GUARD_PY" ] || { echo ">>> ❌ falta $GUARD_PY: no puedo comprobar rutas, el binario NO vale" >&2; exit 3; }
# El código de salida se guarda ANTES de filtrar la salida: en una tubería sin pipefail manda el último
# comando, y un `| sed` habría dado siempre 0 — un candado que no cierra nunca.
GUARD_OUT=$(python3 "$GUARD_PY" "$BIN"); GUARD_RC=$?
printf '%s\n' "$GUARD_OUT" | sed -E 's/accounts=.*/accounts=<redactado>/'
if [ "$GUARD_RC" -ne 0 ]; then
  echo ">>> ❌ el binario lleva rutas de esta máquina (o no se pudo mirar dentro, rc=$GUARD_RC): NO se publica" >&2
  exit 4
fi
sha256sum "$BIN"
