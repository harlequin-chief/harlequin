//! Packages the runtime into the WASM blob a Harlequin node executes (forkless upgrades, SPEC §2.5).
//! Only runs for the `std` build; the `no_std`/wasm compilation of the runtime itself skips it.

fn main() {
    #[cfg(feature = "std")]
    substrate_wasm_builder::WasmBuilder::build_using_defaults();
}
