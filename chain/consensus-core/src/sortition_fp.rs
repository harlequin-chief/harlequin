//! Deterministic fixed-point sortition (i128) — the integer counterpart of `vrf.rs`, so committee
//! election is bit-identical across architectures (a consensus requirement: f64 `exp`/summation is not
//! reproducible). Same algorithm as the f64 path; cross-validated against it within a tiny tolerance.
//!
//! The hard part is the Poisson CDF, which needs `e^{-lam}`. We compute it in fixed-point by range
//! reduction: `e^{-lam} = (e^{-1})^floor(lam) * e^{-frac}`, the integer power by repeated multiply and
//! the fractional part by a short Taylor series (it converges fast because `frac < 1`). No floats, no
//! libm — `no_std`-ready, dependency-free (OPSEC: nothing pulled onto the isolated station).

use crate::sha256::sha256;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// Fixed-point scale, 1e9 — matches `reputation-core` so reputation values feed straight in.
pub const FP: i128 = 1_000_000_000;

/// e^{-1} * FP, rounded. The base of the integer-power range reduction.
const E_INV_FP: i128 = 367_879_441;

/// Anchor for the mode-anchored Poisson weights (1e18). Bigger than FP so the weights carry ~18 digits
/// of dynamic range — enough that even the deep low tail (k ≪ mode) stays representable instead of
/// underflowing to 0, which would otherwise distort the seat count of the very-lowest VRF draws.
const POISSON_ANCHOR: i128 = 1_000_000_000_000_000_000;

#[inline]
fn fp_mul(a: i128, b: i128) -> i128 {
    a * b / FP
}

#[inline]
fn fp_div(a: i128, b: i128) -> i128 {
    a * FP / b
}

/// `e^{-lam}` in fixed-point, `lam_fp = lam * FP >= 0`, result in `[0, FP]`. Range-reduced so it stays
/// accurate for large `lam` (where the naive Taylor series of `e^{-lam}` loses all precision).
pub fn exp_neg_fp(lam_fp: i128) -> i128 {
    if lam_fp <= 0 {
        return FP;
    }
    let n = lam_fp / FP; // integer part of lam
    let f = lam_fp - n * FP; // fractional part in [0, FP)

    // e^{-f} via Taylor: sum_k (-f)^k / k!  (f < 1 -> fast convergence)
    let mut term = FP; // k = 0 term = 1
    let mut sum = FP;
    let mut k: i128 = 1;
    while k <= 20 {
        // term_k = term_{k-1} * (-f) / k   (all fixed-point)
        term = term * (-f) / FP / k;
        sum += term;
        if term == 0 {
            break;
        }
        k += 1;
    }

    // multiply by (e^{-1})^n
    let mut result = sum;
    for _ in 0..n {
        result = fp_mul(result, E_INV_FP);
        if result == 0 {
            break;
        }
    }
    result.clamp(0, FP)
}

/// UNNORMALISED Poisson weights `w[k] ∝ P(X = k)`, indexed by `k` from 0, plus their total. Anchored at
/// the MODE (`w[floor(lam)] = FP`) and recursed outward — `w[k+1] = w[k]·lam/(k+1)`, `w[k-1] = w[k]·k/lam`.
/// The `e^{-lam}` factor of the true PMF is a constant that cancels once we normalise by the total, so it
/// is NEVER computed: that is what kills the large-`lam` underflow that collapsed the old seeded-from-
/// `e^{-lam}` recurrence (the dominant-node seat-cap bug, macroaudit §2.2). The outward loops stop when the
/// weight underflows to 0, so the window self-sizes to the representable mass. `no_std` (alloc), no floats.
fn poisson_weights_fp(lam_fp: i128) -> (Vec<i128>, i128) {
    let m = (lam_fp / FP) as usize; // mode = floor(lam)
    // upward from the mode: indices m, m+1, ...
    let mut up: Vec<i128> = Vec::new();
    let mut w = POISSON_ANCHOR;
    up.push(w);
    let mut k = m as i128;
    loop {
        w = w * lam_fp / FP / (k + 1); // w[k+1] = w[k]·lam/(k+1)
        if w == 0 {
            break;
        }
        up.push(w);
        k += 1;
    }
    // downward from the mode: indices m-1, m-2, ... 0
    let mut down: Vec<i128> = Vec::new();
    let mut w = POISSON_ANCHOR;
    let mut k = m as i128;
    while k > 0 {
        w = w * k * FP / lam_fp; // w[k-1] = w[k]·k/lam
        if w == 0 {
            break; // lower weights are negligible
        }
        down.push(w);
        k -= 1;
    }
    let kmax = m + up.len() - 1;
    let mut wts = alloc::vec![0i128; kmax + 1];
    for (i, &v) in up.iter().enumerate() {
        wts[m + i] = v;
    }
    for (i, &v) in down.iter().enumerate() {
        wts[m - 1 - i] = v;
    }
    let total: i128 = wts.iter().sum();
    (wts, total)
}

/// Poisson CDF `P(X <= j)` for mean `lam` (fixed-point), in `[0, FP]`. Mode-anchored + normalised so it
/// stays accurate for the full `lam` range the sortition uses (no `e^{-lam}` underflow).
pub fn poisson_cdf_fp(j: u32, lam_fp: i128) -> i128 {
    if lam_fp <= 0 {
        return FP;
    }
    let (wts, total) = poisson_weights_fp(lam_fp);
    if total <= 0 {
        return FP;
    }
    let upper = (j as usize).min(wts.len() - 1);
    let partial: i128 = wts[0..=upper].iter().sum();
    (partial * FP / total).min(FP)
}

/// Inverse Poisson CDF: committee seats for a node with expected `lam` and VRF value `value_fp` in
/// `[0, FP)`. `lam_fp <= 0 -> 0` seats. Capped at `max_seats`. Builds the weight window ONCE and walks the
/// cumulative — the smallest `j` with `value < CDF(j)`.
pub fn sortition_seats_fp(value_fp: i128, lam_fp: i128, max_seats: u32) -> u32 {
    if lam_fp <= 0 {
        return 0;
    }
    let (wts, total) = poisson_weights_fp(lam_fp);
    if total <= 0 {
        return 0;
    }
    let mut cum: i128 = 0;
    let mut j = 0;
    while j < max_seats {
        if let Some(&w) = wts.get(j as usize) {
            cum += w;
        }
        if value_fp < (cum * FP / total).min(FP) {
            return j;
        }
        j += 1;
    }
    max_seats
}

/// Deterministic VRF value in `[0, FP)` from (sk, seed): the top 64 bits of SHA-256(`sk|seed`) scaled
/// to fixed-point. Deterministic in (sk, seed) and grind-resistant (a node cannot improve its value
/// without changing its key). Uses fewer bits than the f64 path but is exact integer arithmetic.
pub fn vrf_value_fp(sk: &str, seed: &str) -> i128 {
    let mut input = sk.as_bytes().to_vec();
    input.push(b'|');
    input.extend_from_slice(seed.as_bytes());
    let d = sha256(&input);
    let mut top: u128 = 0;
    for &b in &d[..8] {
        top = (top << 8) | b as u128;
    }
    // value = top / 2^64 * FP, computed without overflow (top < 2^64, FP < 2^30 -> < 2^94).
    ((top * FP as u128) >> 64) as i128
}

/// Elect an epoch committee by reputation-weighted sortition, fully fixed-point. `reputation` is any
/// consistent scale (only the ratio r/total matters); `tau` is the expected total seats. Returns
/// {node: seats} for winners. Deterministic and architecture-independent.
pub fn elect_committee_fp(
    reputation: &BTreeMap<String, i128>,
    secret_keys: &BTreeMap<String, String>,
    seed: &str,
    tau: u32,
) -> BTreeMap<String, u32> {
    let mut committee = BTreeMap::new();
    let total: i128 = reputation.values().map(|&r| if r > 0 { r } else { 0 }).sum();
    if total <= 0 {
        return committee;
    }
    let tau_fp = tau as i128 * FP;
    for (node, &r) in reputation {
        if r <= 0 {
            continue;
        }
        let share = fp_div(r, total); // r/total in FP
        let lam_fp = fp_mul(tau_fp, share); // tau * r/total
        // C-MED1 fix (audit): a reputable node absent from `secret_keys` must be SKIPPED
        // (not eligible — it can't produce a VRF value), never `secret_keys[node]` which would panic
        // IDENTICALLY on every node computing this committee = a synchronised network-wide crash. Fail-safe:
        // exclude the keyless node from the draw. (All current callers build the maps in lockstep; this
        // guards a future/mis-wired caller — e.g. the participation electorate — against a fork-class panic.)
        let value_fp = match secret_keys.get(node) {
            Some(sk) => vrf_value_fp(sk, seed),
            None => continue,
        };
        let seats = sortition_seats_fp(value_fp, lam_fp, 64);
        if seats > 0 {
            committee.insert(node.clone(), seats);
        }
    }
    committee
}

/// The finality-committee seed prefix — ONE definition (node's finality.rs and the pallets both pass
/// it to [`elect_finality_committee`]; a diverging prefix would mean a diverging committee).
pub const FINALITY_COMMITTEE_SEED: &str = "hlq-finality-committee";

/// SINGLE-SOURCE finality-committee election (F2 piece 6, single-source fix): the node's
/// `finality.rs::committee_for_height` AND the on-chain pallets (participation `is_member`, multisig
/// renewal electorate when wired) call THIS — one election, drift impossible. Byte-exact extraction of
/// the node's draw, in its exact order:
/// - `halted` (entrenchment guard §1.4b) or empty reputation → EMPTY committee.
/// - `committed_keys` empty → FALLBACK path (genesis/bootstrap, beacon pipeline unpopulated): every
///   reputable account is eligible, keyed `"sk-{hex(acc)}"`, seed `"{seed_prefix}-{epoch}"`.
/// - beacon populated → only accounts WITH a committed key are eligible, keyed by that committed key
///   (§2.1 anti-grinding: the sortition key is the secret committed an epoch earlier, not a chosen
///   id), seed `"{seed_prefix}-{hex(beacon_seed)}"`.
/// Returns the elected COLD accounts. Hot→cold vote-key resolution stays OUTSIDE (resolve the signer
/// to its account first, then test membership here). All inputs by parameter — zero I/O, `no_std`.
pub fn elect_finality_committee(
    reputation: &[([u8; 32], i128)],
    committed_keys: &[([u8; 32], String)],
    beacon_seed: &[u8; 32],
    epoch: u32,
    tau: u32,
    seed_prefix: &str,
    halted: bool,
) -> alloc::collections::BTreeSet<[u8; 32]> {
    use alloc::format;
    let hex = crate::beacon::hex;
    if halted || reputation.is_empty() {
        return alloc::collections::BTreeSet::new();
    }
    let mut rep_by_id: BTreeMap<String, i128> = BTreeMap::new();
    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    let seed;
    if committed_keys.is_empty() {
        for (acc, rep) in reputation {
            let id = hex(acc);
            keys.insert(id.clone(), format!("sk-{id}"));
            rep_by_id.insert(id, *rep);
        }
        seed = format!("{seed_prefix}-{epoch}");
    } else {
        let kmap: BTreeMap<[u8; 32], &String> =
            committed_keys.iter().map(|(a, k)| (*a, k)).collect();
        for (acc, rep) in reputation {
            if let Some(k) = kmap.get(acc) {
                let id = hex(acc);
                keys.insert(id.clone(), (*k).clone());
                rep_by_id.insert(id, *rep);
            }
        }
        seed = format!("{seed_prefix}-{}", hex(beacon_seed));
    }
    let elected = elect_committee_fp(&rep_by_id, &keys, &seed, tau);
    // LIVENESS FLOOR (F3 wedge + I1): the Poisson draw is size-blind — it can elect
    // NOBODY (testnet τ=4 over 6 uniform accounts → ~1.8% of epochs EMPTY; the fixed epoch seed then
    // wedges finality FOREVER — reproduced at devnet block #32, both F3 runs), and it can seat a
    // committee TOO SMALL to tolerate a single fault (alpha(3)=3/3 → one member down stalls the
    // epoch; I1 catch). Both are liveness holes, not safety features. Floor = min(4, eligible):
    // 4 is the smallest committee whose alpha tolerates 1 faulty member. Below it, re-draw with a
    // salted seed (deterministic — every node and pallet computes the same retries), keeping the
    // LARGEST draw seen (bounded, 32 retries). `halted` / empty-reputation still return EMPTY
    // above: the entrenchment guard's freeze semantics are untouched — this floor only exists
    // where a committee is ALLOWED to exist.
    let eligible = rep_by_id.values().filter(|r| **r > 0).count() as u32;
    let floor = core::cmp::min(4, eligible);
    let mut best = elected;
    let mut retry = 0u32;
    while (best.len() as u32) < floor && retry < 32 {
        retry += 1;
        let redraw = elect_committee_fp(&rep_by_id, &keys, &format!("{seed}-retry-{retry}"), tau);
        if redraw.len() > best.len() {
            best = redraw;
        }
    }
    reputation
        .iter()
        .filter(|(acc, _)| best.contains_key(&hex(acc)))
        .map(|(acc, _)| *acc)
        .collect()
}

#[cfg(test)]
mod finality_committee_tests {
    use super::*;
    use alloc::collections::BTreeSet;
    use alloc::format;
    use alloc::string::ToString;

    fn acc(n: u8) -> [u8; 32] {
        [n; 32]
    }

    #[test]
    fn fallback_path_is_byte_identical_to_the_node_draw() {
        // The pure fn must reproduce EXACTLY what finality.rs::committee_for_height computed inline:
        // ids = hex(acc), keys = "sk-{id}", seed = "{prefix}-{epoch}".
        let reputation = alloc::vec![(acc(1), 100i128), (acc(2), 100), (acc(3), 100)];
        let got = elect_finality_committee(&reputation, &[], &[0u8; 32], 7, 4, "hlq-finality-committee", false);
        // manual reconstruction, the node's original code shape
        let mut rep_map = BTreeMap::new();
        let mut keys = BTreeMap::new();
        for (a, r) in &reputation {
            let id = crate::beacon::hex(a);
            keys.insert(id.clone(), format!("sk-{id}"));
            rep_map.insert(id, *r);
        }
        // Mirror the FULL election algorithm including the liveness floor (min(4, eligible) with
        // salted re-draws keeping the largest): the pinned property is that the extracted fn and
        // this manual reconstruction can never diverge by a single byte.
        let mut best = elect_committee_fp(&rep_map, &keys, "hlq-finality-committee-7", 4);
        let floor = core::cmp::min(4, 3u32); // 3 eligible accounts
        let mut retry = 0u32;
        while (best.len() as u32) < floor && retry < 32 {
            retry += 1;
            let redraw = elect_committee_fp(
                &rep_map,
                &keys,
                &format!("hlq-finality-committee-7-retry-{retry}"),
                4,
            );
            if redraw.len() > best.len() {
                best = redraw;
            }
        }
        let want: BTreeSet<[u8; 32]> = reputation
            .iter()
            .filter(|(a, _)| best.contains_key(&crate::beacon::hex(a)))
            .map(|(a, _)| *a)
            .collect();
        assert_eq!(got, want, "extraction must not change the draw by a single byte");
        assert!(
            got.len() >= 3,
            "3 eligible → floor is all three (a 3-cohort cannot tolerate faults, that's fundamental)"
        );
    }

    #[test]
    fn beacon_path_only_committed_accounts_are_eligible() {
        // §2.1: with the beacon populated, an account WITHOUT a committed key is simply not drawn.
        let reputation = alloc::vec![(acc(1), 100i128), (acc(2), 100)];
        let committed = alloc::vec![(acc(1), "committed-secret-1".to_string())];
        let got = elect_finality_committee(&reputation, &committed, &[9u8; 32], 3, 8, "hlq-finality-committee", false);
        assert!(!got.contains(&acc(2)), "no committed key → not eligible");
        assert!(got.contains(&acc(1)), "the only committed reputable account takes the seats");
    }

    #[test]
    fn halted_guard_seats_nobody() {
        let reputation = alloc::vec![(acc(1), 100i128)];
        let got = elect_finality_committee(&reputation, &[], &[0u8; 32], 1, 4, "x", true);
        assert!(got.is_empty(), "entrenchment halt → empty committee, finality freezes");
    }

    /// REGRESSION — F3 wedge: the six dev accounts (sr25519 //Alice..//Ferdie), uniform
    /// reputation, fallback path, testnet τ=4. The raw Poisson draw for epoch 4's seed elects
    /// NOBODY (verified against elect_committee_fp directly below), which wedged devnet finality
    /// permanently at block #32 in two independent runs. The liveness floor must re-draw with a
    /// salted seed until someone is elected — while the halt path above stays empty.
    #[test]
    fn empty_draw_liveness_floor_devnet_epoch_4() {
        fn unhex(s: &str) -> [u8; 32] {
            let mut a = [0u8; 32];
            for i in 0..32 {
                a[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
            }
            a
        }
        let accounts = [
            "d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d", // Alice
            "8eaf04151687736326c9fea17e25fc5287613693c912909cb226aa4794f26a48", // Bob
            "90b5ab205c6974c9ea841be688864633dc9ca8a357843eeacf2314649965fe22", // Charlie
            "306721211d5404bd9da88e0204360a1a9ab8b87c66c1bc2fcdd37f3c2222cc20", // Dave
            "e659a7a1628cdd93febc04a4e0646ea20e9f5f0ce097d9a05290d4a9e054df4e", // Eve
            "1cbd2d43530a44705ad088af313e18f80b53ef16b36177cd4b77b846f2a5f07c", // Ferdie
        ];
        let reputation: alloc::vec::Vec<([u8; 32], i128)> =
            accounts.iter().map(|h| (unhex(h), 1_000_000i128)).collect();

        // 1. the RAW draw for epoch 4 is empty — the wedge's root cause, pinned so it can't drift.
        let mut rep_map = BTreeMap::new();
        let mut keys = BTreeMap::new();
        for (a, r) in &reputation {
            let id = crate::beacon::hex(a);
            keys.insert(id.clone(), format!("sk-{id}"));
            rep_map.insert(id, *r);
        }
        let raw = elect_committee_fp(&rep_map, &keys, "hlq-finality-committee-4", 4);
        assert!(raw.is_empty(), "epoch 4 raw draw must be the historical empty draw");

        // 2. the floored election seats ≥ min(4, eligible) (salted re-draw) → finality cannot
        //    wedge on an empty epoch NOR on a size-3 committee that tolerates zero faults (I1).
        let got = elect_finality_committee(
            &reputation, &[], &[0u8; 32], 4, 4, "hlq-finality-committee", false,
        );
        assert!(got.len() >= 4, "size floor: ≥ min(4, eligible) so 1 member down never stalls");

        // 3. determinism: two computations agree (every node/pallet must draw identically).
        let again = elect_finality_committee(
            &reputation, &[], &[0u8; 32], 4, 4, "hlq-finality-committee", false,
        );
        assert_eq!(got, again);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vrf::{poisson_cdf_f64_for_test, sortition_seats};

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn exp_neg_matches_f64() {
        for &lam in &[0.0, 0.05, 0.3, 0.7, 1.0, 2.0, 3.5, 5.0, 10.0, 20.0, 40.0] {
            let fp = exp_neg_fp((lam * FP as f64).round() as i128) as f64 / FP as f64;
            let f = (-lam).exp();
            assert!(approx(fp, f, 1e-6 + f * 1e-6), "exp(-{lam}): fp {fp} vs f64 {f}");
        }
    }

    #[test]
    fn poisson_cdf_matches_f64() {
        for &lam in &[0.1, 0.5, 1.0, 3.0, 8.0] {
            let lam_fp = (lam * FP as f64).round() as i128;
            for j in 0..12u32 {
                let fp = poisson_cdf_fp(j, lam_fp) as f64 / FP as f64;
                let f = poisson_cdf_f64_for_test(j, lam);
                assert!(approx(fp, f, 1e-5), "cdf(j={j}, lam={lam}): fp {fp} vs f64 {f}");
            }
        }
    }

    #[test]
    fn sortition_seats_matches_f64() {
        // for matched (value, lam), the fixed-point seat count equals the f64 one.
        for &lam in &[0.5, 1.0, 2.0, 5.0] {
            let lam_fp = (lam * FP as f64).round() as i128;
            for vi in 0..20 {
                let value = vi as f64 / 20.0;
                let value_fp = (value * FP as f64).round() as i128;
                let f = sortition_seats(value, lam, 64);
                let fp = sortition_seats_fp(value_fp, lam_fp, 64);
                assert_eq!(f, fp, "seats(value={value}, lam={lam}): f64 {f} vs fp {fp}");
            }
        }
    }

    #[test]
    fn committee_excludes_sybils_and_sizes_to_tau() {
        let mut reps: BTreeMap<String, i128> = (0..120).map(|i| (format!("h{i}"), FP)).collect();
        let keys: BTreeMap<String, String> =
            reps.keys().map(|n| (n.clone(), format!("sk-{n}"))).collect();
        let c = elect_committee_fp(&reps, &keys, "seed-x", 60);
        let seats: u32 = c.values().sum();
        assert!(seats >= 30 && seats <= 90, "committee ~tau=60, was {seats}");

        for i in 0..2000 {
            reps.insert(format!("s{i}"), FP / 1_000_000); // reputation ~0
        }
        let keys2: BTreeMap<String, String> =
            reps.keys().map(|n| (n.clone(), format!("sk-{n}"))).collect();
        let c2 = elect_committee_fp(&reps, &keys2, "seed-y", 60);
        let sybils = c2.keys().filter(|n| n.starts_with('s')).count();
        assert_eq!(sybils, 0, "sybils must not enter the committee, got {sybils}");
    }

    #[test]
    fn keyless_reputable_node_is_skipped_not_panicked() {
        // C-MED1 (audit): a reputable node with NO secret_keys entry must be EXCLUDED from the
        // draw, never `secret_keys[node]` (an Index panic that fires identically on every node computing
        // this committee = a fork-class network halt). This constructs exactly that mismatch.
        let reps: BTreeMap<String, i128> = (0..10).map(|i| (format!("h{i}"), FP)).collect();
        // keys for everyone EXCEPT h7
        let keys: BTreeMap<String, String> =
            reps.keys().filter(|n| *n != "h7").map(|n| (n.clone(), format!("sk-{n}"))).collect();
        let c = elect_committee_fp(&reps, &keys, "seed-z", 60); // must NOT panic
        assert!(!c.contains_key("h7"), "keyless node must be excluded from the committee");
    }

    /// Helper: worst |f64 − fixed-point| seat gap over a fine VRF-value grid, for a given lam.
    fn max_seat_gap(lam: f64) -> i64 {
        let lam_fp = (lam * FP as f64).round() as i128;
        let mut max_gap = 0i64;
        // 1..1000: exclude the exact 0 and 1 endpoints — the VRF never emits them, and at the deep tail
        // the f64 oracle's sub-1e-18 CDF is below any representable fixed-point resolution.
        for vi in 1..1000 {
            let value = vi as f64 / 1000.0;
            let value_fp = (value * FP as f64).round() as i128;
            let f = sortition_seats(value, lam, 64) as i64;
            let fp = sortition_seats_fp(value_fp, lam_fp, 64) as i64;
            max_gap = max_gap.max((f - fp).abs());
        }
        max_gap
    }

    #[test]
    fn sortition_matches_f64_across_the_full_lam_range() {
        // Macro-audit 2.2 — FIXED (campaña estrés tick 11, mode-anchored Poisson). The fixed-point
        // sortition now tracks the f64 oracle to within 2 seats across the WHOLE VRF-value range for the
        // full span of per-node lam the consensus can see (lam = tau * r/total, capped by max_seats). The
        // old version only held for lam <= 14 and collapsed to the seat cap for lam >~ 21; this is the
        // regression guard that the fix holds end to end.
        for &lam in &[0.5, 1.0, 2.0, 5.0, 8.0, 12.0, 16.0, 21.0, 30.0, 40.0, 50.0, 60.0] {
            assert!(max_seat_gap(lam) <= 2, "lam={lam}: seat gap {} (fix should keep it tiny everywhere)", max_seat_gap(lam));
        }
    }

    #[test]
    fn large_lam_no_longer_collapses_to_the_seat_cap() {
        // The dominant-node defect (was macroaudit §2.2 🟠→🔴) is FIXED by the mode-anchored Poisson:
        // the e^{-lam} factor cancels in normalisation, so the CDF no longer underflows to 0 for large
        // lam. A high-lam node now gets a Poisson-distributed seat count that SPREADS with the VRF value
        // (proportionality + rotation restored, Art. VI) instead of the cap for every value.
        let lam_fp = (40.0 * FP as f64) as i128;
        // the CDF is a proper non-degenerate distribution: 0 at the low tail, ~full at the high tail.
        assert!(poisson_cdf_fp(20, lam_fp) < FP / 100, "low tail near 0, was {}", poisson_cdf_fp(20, lam_fp));
        assert!(poisson_cdf_fp(60, lam_fp) > FP * 99 / 100, "high tail near 1, was {}", poisson_cdf_fp(60, lam_fp));
        // seats now SPREAD with the VRF value — a low value yields few seats, a high value many — not the cap.
        assert!(sortition_seats_fp(FP / 100, lam_fp, 64) < 35, "low VRF value must NOT get the cap");
        assert!(sortition_seats_fp(99 * FP / 100, lam_fp, 64) > 45, "high VRF value gets more seats");
        // and it tracks the f64 oracle at this previously-broken lam.
        assert!(max_seat_gap(40.0) <= 2, "lam=40 must now match f64, gap was {}", max_seat_gap(40.0));
    }

    /// Helper: (best seats found by grinding `trials` candidate keys, average seats) for a fixed lam+seed.
    fn grind_seats(lam: f64, seed: &str, trials: u32) -> (u32, f64) {
        let lam_fp = (lam * FP as f64).round() as i128;
        let (mut max_seats, mut sum) = (0u32, 0u64);
        for i in 0..trials {
            let s = sortition_seats_fp(vrf_value_fp(&format!("sk-attacker{i}"), seed), lam_fp, 64);
            max_seats = max_seats.max(s);
            sum += s as u64;
        }
        (max_seats, sum as f64 / trials as f64)
    }

    #[test]
    fn known_defect_vrf_is_grindable_but_reputation_anchor_holds() {
        // KNOWN DEFECT (campaña estrés tick 7, macroaudit §2.1): the sortition uses a SIMULATED VRF —
        // value = SHA-256(sk|seed) — and the sk is DERIVED FROM A FREELY-CHOSEN id (protocol-core:
        // sk-{id}; Art. VII lets a member pick their name). So a node can GRIND its id offline against a
        // predictable epoch seed to lift its committee seats well above its fair share. This pins:
        //   * the exploit: grinding ~400 ids ~doubles+ the seats vs the honest average (which tracks lam);
        //   * the boundary that saves it from being catastrophic: a node with ZERO reputation (lam=0)
        //     wins 0 seats no matter how hard it grinds — the reputation anchor (not the VRF) is the real
        //     Sybil defence. Grinding only helps a node that ALREADY earned reputation, and only one-shot
        //     per predictable seed (the id is a fixed identity; the epoch folds into the seed).
        // The fix is macroaudit §2.1: a real ECVRF (committed keypair, not a choosable string) + a beacon
        // unpredictable until after keys are committed. This test flips when that lands.
        let seed = "beacon|epoch1";
        for &lam in &[2.0, 5.0, 8.0] {
            let (best, avg) = grind_seats(lam, seed, 400);
            assert!((avg - lam).abs() < lam * 0.3, "sortition must be fair on average: lam={lam} avg={avg}");
            assert!(best as f64 > avg * 1.8, "grinding should lift seats well above the fair share: lam={lam} best={best} avg={avg}");
        }
        // the anchor: zero reputation -> zero seats, grind as you like.
        let (best0, _) = grind_seats(0.0, seed, 400);
        assert_eq!(best0, 0, "a zero-reputation node wins no seats however much it grinds");
    }

    #[test]
    fn deterministic_across_runs() {
        let reps: BTreeMap<String, i128> = (0..50).map(|i| (format!("h{i}"), FP)).collect();
        let keys: BTreeMap<String, String> =
            reps.keys().map(|n| (n.clone(), format!("sk-{n}"))).collect();
        let a = elect_committee_fp(&reps, &keys, "beacon|epoch3", 25);
        let b = elect_committee_fp(&reps, &keys, "beacon|epoch3", 25);
        assert_eq!(a, b);
    }
}
