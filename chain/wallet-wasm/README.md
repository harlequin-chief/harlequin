# wallet-wasm — the browser wallet

The wallet the site runs in your browser: your mask (an sr25519 key) is created, sealed (Argon2id +
XChaCha20-Poly1305) and used **on your device only**. This crate is a thin `wasm-bindgen` wrapper over
`../wallet-core`, which holds all the cryptography.

Build: `./build-wasm.sh` (needs the `wasm32-unknown-unknown` target and `wasm-bindgen-cli` at the version pinned in
`Cargo.lock`). The script remaps every build path to `/hlq-build/...` and refuses to finish if any path of the
building machine survives inside the wasm.

The file served at `/pkg/wallet_wasm_bg.wasm` and `/villa/entrar/wallet/wallet_wasm_bg.wasm` since 2026-10-01 has
sha256 `caa4010ef739f30d8afb393fb9bb9ecae885a19a1487528d8a4bdaa26a2a70eb` and was built from this source.
