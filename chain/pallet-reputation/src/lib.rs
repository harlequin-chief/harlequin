//! Harlequin reputation pallet — on-chain inputs, reputation DERIVED.
//!
//! The chain stores the **verifiable inputs** (objective evidence per suit + the vouch graph), never an
//! asserted reputation number. Reputation is recomputed from those inputs by `reputation-core` on the
//! deterministic fixed-point path — so every node can re-derive and reject a wrong result (no trusted
//! oracle of who is reputable). See `../PALLET-DESIGN.md`.
//!
//! Implemented: storage (evidence, vouch graph, reputation snapshot, epoch, telemetry), the input calls
//! (`submit_evidence`, `vouch`/`revoke_vouch` with a sublinear all-suits quota), the **automatic** epoch
//! recompute (`on_initialize` at each `EpochLength` boundary — NO privileged trigger, no king),
//! `report_fraud` (cascade slashing), and a genesis cohort.
//!
//! ## Pre-mainnet notes (honest limitations)
//! - **Weights are placeholders, not benchmarked.** They must be benchmarked before mainnet. The HEAVY
//!   work — the epoch `recompute_reputation` (EigenTrust over the whole graph) run in `on_initialize`, and
//!   `report_fraud` (`cascade_slash`, which scans `Vouches` once per hop) — must move to an **offchain
//!   worker** (verify-by-recompute) before it runs on untrusted/large state (PALLET-DESIGN "Epoch flow").
//!   The recompute has no extrinsic and no origin (it is the chain's own clock); `report_fraud`/
//!   `submit_evidence` are gated to PRIVILEGED origins; the only signed-user call, `vouch`, is bounded by
//!   `MaxVouches`.
//! - **`cascade_slash`** attenuates the loss by `SponsorLiability` (a mutable dial) per hop and is bounded by `depth`, but a *cyclic* vouch graph
//!   can slash a node more than once on the way up. Bounded (depth + halving → 0); a future version may
//!   track visited nodes.
//! - **Evidence enters `reputation-core` as `f64`** (then to fixed-point): exact for realistic
//!   magnitudes; astronomically large `u128` evidence would lose precision / saturate. Consider a cap.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

/// The four reputation dimensions = the four suits of Harlequin (LORE.md, canon). Fixed set.
#[derive(
    codec::Encode,
    codec::Decode,
    codec::DecodeWithMemTracking,
    codec::MaxEncodedLen,
    scale_info::TypeInfo,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Debug,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum Suit {
    /// ♦ commerce / ambition
    Commerce,
    /// ♣ technical contribution / freedom
    Technical,
    /// ♠ judicial function / struggle
    Judicial,
    /// ♥ governance / love
    Governance,
}

/// Per-epoch network telemetry written on-chain (SPEC §5c). Any node serves it to the public panel —
/// no central dashboard. The data the network publishes about itself, including whether power is
/// concentrating (`top_reputation`).
#[derive(
    codec::Encode,
    codec::Decode,
    codec::DecodeWithMemTracking,
    codec::MaxEncodedLen,
    scale_info::TypeInfo,
    Clone,
    PartialEq,
    Eq,
    Debug,
    Default,
)]
pub struct EpochReport {
    /// participants this epoch (anyone with evidence or in the vouch graph).
    pub members: u32,
    /// participants with positive consensus reputation (the conservative min across suits > 0).
    pub active_members: u32,
    /// the single highest consensus reputation, raw fixed-point — the entrenchment watch.
    pub top_reputation: i128,
}

impl Suit {
    /// All four suits, canonical order.
    pub const ALL: [Suit; 4] = [Suit::Commerce, Suit::Technical, Suit::Judicial, Suit::Governance];

    /// The reputation-core dimension name. MUST match `reputation_core::DIMENSIONS` exactly.
    pub const fn dim_name(&self) -> &'static str {
        match self {
            Suit::Commerce => "commerce",
            Suit::Technical => "technical_contribution",
            Suit::Judicial => "judicial_function",
            Suit::Governance => "governance",
        }
    }
}

#[frame::pallet]
pub mod pallet {
    use super::{EpochReport, Suit};
    use frame::prelude::*;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The aggregate event type of the runtime.
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Origin allowed to record validated evidence. Evidence is NOT self-asserted: what counts as
        /// proven work is pluggable (a settled-deal proof, a completed-job attestation, an oracle, a
        /// governance body…), exactly like personhood is pluggable in SPEC §1.5. The pallet does not
        /// hard-code it — it trusts this origin to have verified the proof.
        type EvidenceOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Origin allowed to act on a proven-guilty fraud verdict (the justice function, §4). Like
        /// `EvidenceOrigin`, the VERDICT is pluggable (a jury pallet, governance…); this pallet only
        /// carries out the slashing once something it trusts has ruled.
        type JusticeOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Max live vouches a single account can hold (bounds storage / PoV).
        #[pallet::constant]
        type MaxVouches: Get<u32>;

        /// `k` in the sublinear sponsorship quota `floor(k · log2(1 + reputation))` (§1.5c): higher = more
        /// vouches per unit of standing. Decreasing returns either way, so nobody monopolises sponsorship.
        #[pallet::constant]
        type VouchQuotaK: Get<u32>;

        /// Per-epoch evidence retention ρ (§1.7 decay / Art. VI anti-entrenchment): at every epoch each
        /// account's evidence is multiplied by this, and the rest (1−ρ) evaporates. Applies to EVERYONE,
        /// the founder included — there is **no genesis exemption** (pre-freeze audit, issue #3): standing
        /// is earned continuously and fades when you stop contributing, so "the pioneers' advantage
        /// dilutes" (Art. VI) is true in code, not just on paper. The exact rate is a tunable dial chosen
        /// from the offline decay sweep (curves A/B/C); a value near 1 decays slowly, a smaller one fast.
        #[pallet::constant]
        type EvidenceRetention: Get<Permill>;

        /// Blocks per epoch. The reputation recompute runs **automatically** at every epoch boundary
        /// (`on_initialize`), with NO privileged trigger — there is no king who decides when reputation is
        /// recounted. Each node runs the same deterministic recompute over the on-chain inputs, so a wrong
        /// result is rejected (verify-by-recompute). The real value is fixed pre-mainnet with the consensus
        /// cadence (#30); for scale the heavy compute moves to an offchain worker (PALLET-DESIGN), which does
        /// not change the no-king property.
        #[pallet::constant]
        type EpochLength: Get<BlockNumberFor<Self>>;

        /// Fraction of a protégé's slashed loss that each DIRECT sponsor answers for, cascaded once per
        /// hop up the vouch chain (§1.5c, "you answer for whom you bring in"). The dial that balances
        /// vouching: too high and sponsoring is pure downside, so nobody vouches and the trust graph never
        /// forms — which would strangle the very growth the dilution of Art. VI depends on (pre-freeze
        /// audit, issue #4); too low and there is no skin in the game. MUTABLE calibration. Any value < 1
        /// keeps the cascade geometrically convergent (bounded by `depth`).
        #[pallet::constant]
        type SponsorLiability: Get<Permill>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Objective verifiable evidence per (account, suit) — the bootstrap anchor of reputation (§1.3a).
    #[pallet::storage]
    pub type Evidence<T: Config> =
        StorageDoubleMap<_, Blake2_128Concat, T::AccountId, Blake2_128Concat, Suit, u128, ValueQuery>;

    /// The vouch graph (§1.3b): for each account, the edges it casts — (target, suit, weight). Putting
    /// reputation at stake on someone; persistent liability lives alongside (§1.5c, next increment).
    #[pallet::storage]
    pub type Vouches<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        BoundedVec<(T::AccountId, Suit, u32), T::MaxVouches>,
        ValueQuery,
    >;

    /// Last epoch's computed reputation per (account, suit), raw fixed-point i128 (what consensus and
    /// justice READ). Written by the epoch recompute (next increment); read-only between epochs.
    #[pallet::storage]
    pub type ReputationSnapshot<T: Config> =
        StorageDoubleMap<_, Blake2_128Concat, T::AccountId, Blake2_128Concat, Suit, i128, ValueQuery>;

    /// Current epoch number (drives recompute cadence + committee rotation, SPEC §2.2).
    #[pallet::storage]
    pub type Epoch<T> = StorageValue<_, u64, ValueQuery>;

    /// Last epoch's network telemetry (SPEC §5c) — what the nodes serve to the public panel.
    #[pallet::storage]
    pub type LastReport<T> = StorageValue<_, EpochReport, OptionQuery>;

    /// Genesis: seed the founding cohort's objective evidence (§1.4), so the chain boots with members
    /// who already have standing to bootstrap trust. Nothing here is reputation — only the evidence
    /// inputs; reputation is still DERIVED on the first epoch recompute.
    #[pallet::genesis_config]
    #[derive(frame::prelude::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        /// `(account, suit, evidence_amount)` granted at genesis.
        pub evidence: alloc::vec::Vec<(T::AccountId, Suit, u128)>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            for (who, suit, amount) in &self.evidence {
                Evidence::<T>::insert(who, *suit, *amount);
            }
            // Derive the founding cohort's reputation at block 0 so the chain boots with a real,
            // reputation-weighted committee (not a fallback). Reputation is still DERIVED, never
            // asserted — genesis only seeds the evidence, the snapshot is computed from it.
            if !self.evidence.is_empty() {
                let _ = Pallet::<T>::recompute_reputation();
            }
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Validated evidence was recorded for `who` in `suit` (new cumulative total).
        EvidenceRecorded { who: T::AccountId, suit: Suit, total: u128 },
        /// `voucher` vouched for `target` in `suit` with `weight`.
        Vouched { voucher: T::AccountId, target: T::AccountId, suit: Suit, weight: u32 },
        /// `voucher` revoked its vouch(es) for `target` in `suit`.
        VouchRevoked { voucher: T::AccountId, target: T::AccountId, suit: Suit },
        /// A new epoch began, with its network telemetry (members, active members).
        EpochAdvanced { epoch: u64, members: u32, active_members: u32 },
        /// A proven fraud slashed `culprit` in `suit`; the loss cascaded up the vouch chain.
        FraudSlashed { culprit: T::AccountId, suit: Suit, loss: u128 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Nobody vouches for themselves (§1.3b).
        SelfVouch,
        /// The caller has reached its vouch capacity.
        TooManyVouches,
        /// Vouch weight must be non-zero.
        ZeroWeight,
        /// The caller's reputation does not yet earn the right to cast another vouch (§1.5c quota).
        VouchQuotaReached,
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// No-king epoch advance: at every epoch boundary the chain recounts reputation by ITSELF — no
        /// privileged origin presses a button (that would be a single point of control in a chain meant to
        /// have none). Deterministic, so every node recomputes the same result and rejects a wrong one.
        fn on_initialize(n: BlockNumberFor<T>) -> Weight {
            let len = T::EpochLength::get();
            if !n.is_zero() && !len.is_zero() && (n % len).is_zero() {
                Self::run_epoch();
            }
            // Placeholder weight (recompute is heavy; benchmark + offchain-worker move pre-mainnet).
            Weight::from_parts(1_000_000_000, 0)
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Record validated objective evidence for `who` in `suit` (adds to the running total). Gated by
        /// `EvidenceOrigin` — the pluggable proof authority (§1.3a). Not self-asserted.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn submit_evidence(
            origin: OriginFor<T>,
            who: T::AccountId,
            suit: Suit,
            amount: u128,
        ) -> DispatchResult {
            T::EvidenceOrigin::ensure_origin(origin)?;
            let total = Evidence::<T>::mutate(&who, suit, |e| {
                *e = e.saturating_add(amount);
                *e
            });
            Self::deposit_event(Event::EvidenceRecorded { who, suit, total });
            Ok(())
        }

        /// Vouch for `target` in `suit` with `weight` — put reputation at stake on them (§1.3b).
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn vouch(
            origin: OriginFor<T>,
            target: T::AccountId,
            suit: Suit,
            weight: u32,
        ) -> DispatchResult {
            let voucher = ensure_signed(origin)?;
            ensure!(voucher != target, Error::<T>::SelfVouch);
            ensure!(weight > 0, Error::<T>::ZeroWeight);
            // sublinear sponsorship quota by reputation (§1.5c): you earn the right to vouch.
            let current = Vouches::<T>::get(&voucher).len() as u64;
            ensure!(current < Self::vouch_quota_of(&voucher), Error::<T>::VouchQuotaReached);
            Vouches::<T>::try_mutate(&voucher, |edges| {
                edges
                    .try_push((target.clone(), suit, weight))
                    .map_err(|_| Error::<T>::TooManyVouches)
            })?;
            Self::deposit_event(Event::Vouched { voucher, target, suit, weight });
            Ok(())
        }

        /// Revoke all of the caller's vouches for `target` in `suit`.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn revoke_vouch(
            origin: OriginFor<T>,
            target: T::AccountId,
            suit: Suit,
        ) -> DispatchResult {
            let voucher = ensure_signed(origin)?;
            Vouches::<T>::mutate(&voucher, |edges| {
                edges.retain(|(t, s, _)| !(*t == target && *s == suit));
            });
            Self::deposit_event(Event::VouchRevoked { voucher, target, suit });
            Ok(())
        }

        /// Act on a proven-guilty fraud verdict (§1.7): slash the culprit's evidence in `suit` by
        /// `loss`, and cascade the liability up the vouch chain — each sponsor up to `depth` hops loses
        /// `SponsorLiability` of what their protégé lost (§1.5c, *you answer for whom you bring in*). The
        /// share is a mutable dial (issue #4): enough skin in the game to deter reckless vouching, not so
        /// much that sponsoring is pure downside. Slashing hits the
        /// EVIDENCE inputs, so the lower reputation persists through the next recompute. Gated by
        /// `JusticeOrigin` (the pluggable verdict authority).
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(100_000, 0))]
        pub fn report_fraud(
            origin: OriginFor<T>,
            culprit: T::AccountId,
            suit: Suit,
            loss: u128,
            depth: u8,
        ) -> DispatchResult {
            T::JusticeOrigin::ensure_origin(origin)?;
            Self::cascade_slash(&culprit, suit, loss, depth);
            Self::deposit_event(Event::FraudSlashed { culprit, suit, loss });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Per-account **consensus reputation** = the conservative MINIMUM across the four suits (§1.2b),
        /// raw fixed-point i128, for every account with a positive value. This is what the node's
        /// Woven-Trust sortition weights on; exposed to the node through the `HarlequinConsensusApi`
        /// runtime API. Reads the last epoch's snapshot (read-only between epochs).
        pub fn consensus_reputation() -> alloc::vec::Vec<(T::AccountId, i128)> {
            use alloc::collections::BTreeSet;
            // Candidate accounts: any that appear in the snapshot.
            let mut accounts: BTreeSet<T::AccountId> = BTreeSet::new();
            for (acc, _suit, _r) in ReputationSnapshot::<T>::iter() {
                accounts.insert(acc);
            }
            let mut out = alloc::vec::Vec::new();
            for acc in accounts {
                let mut min_raw = i128::MAX;
                for suit in Suit::ALL {
                    let r = ReputationSnapshot::<T>::get(&acc, suit);
                    if r < min_raw {
                        min_raw = r;
                    }
                }
                if min_raw > 0 {
                    out.push((acc, min_raw));
                }
            }
            out
        }

        /// Live-vouch quota of `who` (§1.5c): `floor(k · log2(1 + reputation))`, where reputation is the
        /// CONSERVATIVE MINIMUM across the four suits (§1.2b) — you cannot sponsor on the strength of one
        /// suit alone; you must be trustworthy in all four. Reads the last epoch's snapshot.
        pub fn vouch_quota_of(who: &T::AccountId) -> u64 {
            let mut min_raw = i128::MAX;
            for suit in Suit::ALL {
                let r = ReputationSnapshot::<T>::get(who, suit);
                if r < min_raw {
                    min_raw = r;
                }
            }
            if min_raw <= 0 {
                return 0;
            }
            // The snapshot is raw fixed-point (network mass ≈ FP_SCALE); map it to the §1 display scale
            // (×1000) so the log2 lands in a meaningful range.
            let agg_fp = min_raw.saturating_mul(1000);
            let k_fp = (T::VouchQuotaK::get() as i128).saturating_mul(reputation_core::FP_SCALE);
            reputation_core::vouch::vouch_quota_fp(agg_fp, k_fp)
        }

        /// Slash `agent`'s evidence in `suit` by `amount` and cascade the liability up the vouch graph:
        /// every account that vouched for `agent` in `suit` loses `SponsorLiability` of it, level by level,
        /// until `depth` runs out (§1.5c, *you answer for whom you bring in*). Breadth-first with a **visited set** so
        /// a CYCLIC vouch graph cannot slash anyone twice; each member is hit once, by its shallowest
        /// (largest) path. Deterministic (BTreeSet/`Vouches::iter` ordered).
        fn cascade_slash(agent: &T::AccountId, suit: Suit, amount: u128, depth: u8) {
            use alloc::collections::BTreeSet;
            use alloc::vec::Vec;
            let mut visited: BTreeSet<T::AccountId> = BTreeSet::new();
            let mut level: Vec<(T::AccountId, u128)> = alloc::vec![(agent.clone(), amount)];
            let mut remaining = depth;
            loop {
                let mut next: Vec<(T::AccountId, u128)> = Vec::new();
                for (who, amt) in level.drain(..) {
                    if amt == 0 || visited.contains(&who) {
                        continue;
                    }
                    visited.insert(who.clone());
                    Evidence::<T>::mutate(&who, suit, |e| *e = e.saturating_sub(amt));
                    if remaining == 0 {
                        continue;
                    }
                    let passes_on = T::SponsorLiability::get().mul_floor(amt); // configured share per hop
                    if passes_on == 0 {
                        continue;
                    }
                    for (voucher, edges) in Vouches::<T>::iter() {
                        if !visited.contains(&voucher)
                            && edges.iter().any(|(t, s, _)| t == &who && *s == suit)
                        {
                            next.push((voucher, passes_on));
                        }
                    }
                }
                if next.is_empty() || remaining == 0 {
                    break;
                }
                level = next;
                remaining -= 1;
            }
        }

        /// Close one epoch: age all evidence (§1.7 leaky integrator), recompute every member's reputation
        /// from the on-chain inputs, write the snapshot + telemetry, and bump the epoch counter. Called
        /// automatically by `on_initialize` at each epoch boundary — never by an extrinsic, so no privileged
        /// actor controls the recount. Genesis is NOT affected (it calls `recompute_reputation` directly, so
        /// the founding evidence first ages on the move into epoch 1, never before standing is earned).
        pub(crate) fn run_epoch() {
            Self::decay_evidence();
            let report = Self::recompute_reputation().unwrap_or_default();
            LastReport::<T>::put(report.clone());
            let epoch = Epoch::<T>::mutate(|e| {
                *e = e.saturating_add(1);
                *e
            });
            Self::deposit_event(Event::EpochAdvanced {
                epoch,
                members: report.members,
                active_members: report.active_members,
            });
        }

        /// Whether `a` and `b` have a DIRECT trust tie — a vouch edge `a→b` or `b→a` in ANY suit. The
        /// justice pallet's interest test (Art. IX) uses this: a direct truster or trustee of a party must
        /// not sit on its jury. Conservative depth-1 relation (reads only the two accounts' edges, cheap);
        /// deeper ties (vouch-of-vouch, shared cluster) are a documented future tightening.
        pub fn is_related(a: &T::AccountId, b: &T::AccountId) -> bool {
            if a == b {
                return true;
            }
            if Vouches::<T>::get(a).iter().any(|(t, _, _)| t == b) {
                return true;
            }
            Vouches::<T>::get(b).iter().any(|(t, _, _)| t == a)
        }

        /// Relation depth (1 or 2) from `who` to the NEAREST of `parties` in the undirected vouch graph,
        /// or `None` if independent within depth 2 (or the search hits the `max_visit` cost cap). depth-1
        /// = a direct tie (`is_related`, excluded outright at the jury draw); depth-2 = a SHARED NEIGHBOUR
        /// (the 2-hop ring of SPEC §4i-(8) that evades depth-1 exclusion — quantified residual: a ring of
        /// loyal reputed members woven at distance 2). Justice **weight-penalises** depth-2 jurors instead
        /// of cutting them (a hard depth-2 cut shrinks the pool ~35% and opens pool-poisoning). Mirrors the
        /// cross-validated `reputation-core::VouchRegistry::nearest_party_distance`.
        ///
        /// **Bounded (anti-DoS):** the in-edge search reads at most `max_visit` accounts; a party
        /// with a huge fan-out cannot make opening a case unbounded — a capped-out search returns `None`
        /// (no penalty, never a false relation). Deterministic (`Vouches::iter` order + `BTreeSet`).
        pub fn relation_depth(
            who: &T::AccountId,
            parties: &[T::AccountId],
            max_visit: u32,
        ) -> Option<u32> {
            use alloc::collections::BTreeSet;
            // depth 1: a direct tie to any party (these are already excluded from the draw).
            if parties.iter().any(|p| Self::is_related(who, p)) {
                return Some(1);
            }
            // depth 2: who shares an undirected neighbour with a party. Build who's neighbour set, bounded.
            let mut neighbours: BTreeSet<T::AccountId> = BTreeSet::new();
            let mut budget = max_visit;
            // out-edges of `who` (who → M): cheap, bounded by who's own vouch list.
            for (t, _, _) in Vouches::<T>::get(who) {
                if budget == 0 {
                    return None;
                }
                budget -= 1;
                neighbours.insert(t);
            }
            // in-edges of `who` (M → who): scan accounts whose vouch list contains `who`. Cost-capped so a
            // huge fan-out cannot inflate the per-case cost (the DoS bound).
            for (m, edges) in Vouches::<T>::iter() {
                if budget == 0 {
                    return None;
                }
                budget -= 1;
                if &m != who && edges.iter().any(|(t, _, _)| t == who) {
                    neighbours.insert(m);
                }
            }
            // who is depth-2 to a party iff some neighbour is directly related to that party.
            if neighbours
                .iter()
                .any(|m| parties.iter().any(|p| m != p && Self::is_related(m, p)))
            {
                return Some(2);
            }
            None
        }

        /// Carry out a jury verdict's reputational consequence: slash `culprit`'s evidence in `suit` by
        /// `loss`, cascading the liability up the vouch chain (§1.7). Public entry for the justice pallet's
        /// verdict handler — the proven verdict IS the authority, so no extrinsic origin is needed here (the
        /// gating lives in the justice pallet: only a quorum of a non-interested jury reaches this). `depth`
        /// bounds the cascade.
        pub fn slash_for_verdict(culprit: &T::AccountId, suit: Suit, loss: u128, depth: u8) {
            Self::cascade_slash(culprit, suit, loss, depth);
        }

        /// Age every account's evidence by one epoch: `E ← floor(ρ · E)` for all `(account, suit)`, the
        /// founder included (§1.7 decay, Art. VI anti-entrenchment). Evidence is a **leaky integrator**:
        /// validated work adds at full weight (`submit_evidence`), old work evaporates here. So reputation
        /// rises and HOLDS only while you keep contributing, and DECAYS when you stop — yesterday's power
        /// cannot shield tomorrow's, and the pioneers' advantage dilutes. Entries that reach zero are
        /// removed (no storage bloat). Deterministic: snapshots the keys first (ordered `iter`), then
        /// rewrites, so mutation never disturbs the iteration.
        fn decay_evidence() {
            let rho = T::EvidenceRetention::get();
            let entries: alloc::vec::Vec<(T::AccountId, Suit, u128)> = Evidence::<T>::iter().collect();
            for (who, suit, e) in entries {
                let decayed = rho.mul_floor(e);
                if decayed == 0 {
                    Evidence::<T>::remove(&who, suit);
                } else if decayed != e {
                    Evidence::<T>::insert(&who, suit, decayed);
                }
            }
        }

        /// Recompute every member's reputation from the on-chain inputs (`Evidence` + `Vouches`) and
        /// overwrite `ReputationSnapshot`. Deterministic — it runs `reputation-core` on the fixed-point
        /// path, so every node derives the same numbers and can reject a wrong snapshot. The chain
        /// stores INPUTS, never an asserted reputation (no oracle).
        ///
        /// HEAVY (EigenTrust over the whole graph): this in-runtime version is for testnet/triggering;
        /// production runs it in an offchain worker and verifies by recompute.
        pub fn recompute_reputation() -> Option<EpochReport> {
            use alloc::collections::{BTreeMap, BTreeSet};
            use alloc::string::ToString;
            use alloc::vec::Vec;
            use reputation_core::{reputation_dimension_fully_fixed_fp, Agent, Params, TrustGraph};

            // 1. participants: anyone with evidence, who vouches, or who is vouched for.
            let mut set: BTreeSet<T::AccountId> = BTreeSet::new();
            for (acc, _suit, _v) in Evidence::<T>::iter() {
                set.insert(acc);
            }
            for (acc, edges) in Vouches::<T>::iter() {
                set.insert(acc);
                for (target, _s, _w) in edges.into_iter() {
                    set.insert(target);
                }
            }
            let accounts: Vec<T::AccountId> = set.into_iter().collect();
            if accounts.is_empty() {
                return None;
            }
            // reputation-core keys agents by short string ids = their index in `accounts`.
            let mut idx: BTreeMap<T::AccountId, usize> = BTreeMap::new();
            for (i, a) in accounts.iter().enumerate() {
                idx.insert(a.clone(), i);
            }

            // 2. agents (evidence per suit)
            let mut agents: Vec<Agent> = Vec::with_capacity(accounts.len());
            for (i, acc) in accounts.iter().enumerate() {
                let mut agent = Agent::new(&i.to_string());
                for suit in Suit::ALL {
                    let ev = Evidence::<T>::get(acc, suit);
                    if ev > 0 {
                        agent = agent.with_evidence(suit.dim_name(), ev as f64);
                    }
                }
                agents.push(agent);
            }

            // 3. trust graph (vouches)
            let mut graph = TrustGraph::new();
            for (acc, edges) in Vouches::<T>::iter() {
                let si = match idx.get(&acc) {
                    Some(s) => *s,
                    None => continue,
                };
                for (target, suit, weight) in edges.into_iter() {
                    if let Some(ti) = idx.get(&target) {
                        graph.attest(&si.to_string(), &ti.to_string(), suit.dim_name(), weight as f64);
                    }
                }
            }

            // 4. compute per suit (deterministic fixed-point), overwrite the snapshot, and track each
            //    member's CONSERVATIVE MINIMUM across suits (§1.2b) for the telemetry.
            let params = Params { community: true, in_concentration: true, ..Default::default() };
            let mut min_by: Vec<i128> = alloc::vec![i128::MAX; accounts.len()];
            for suit in Suit::ALL {
                let rep = reputation_dimension_fully_fixed_fp(&agents, &graph, suit.dim_name(), &params);
                for (id, value) in rep.into_iter() {
                    if let Ok(i) = id.parse::<usize>() {
                        if let Some(acc) = accounts.get(i) {
                            ReputationSnapshot::<T>::insert(acc, suit, value);
                            if value < min_by[i] {
                                min_by[i] = value;
                            }
                        }
                    }
                }
            }

            // 5. network telemetry (SPEC §5c).
            let active_members = min_by.iter().filter(|&&m| m > 0).count() as u32;
            let top_reputation =
                min_by.iter().copied().filter(|&m| m != i128::MAX).max().unwrap_or(0).max(0);
            Some(EpochReport { members: accounts.len() as u32, active_members, top_reputation })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_reputation;
    use crate::{Epoch, Error, Evidence, LastReport, ReputationSnapshot, Suit};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Reputation: pallet_reputation,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    parameter_types! {
        // ρ = 0.50 per epoch for tests: evidence halves each epoch, so decay is easy to assert.
        pub const TestRetention: Permill = Permill::from_percent(50);
        // 50% sponsor liability per hop in tests: keeps the existing cascade arithmetic (½ per hop).
        pub const TestLiability: Permill = Permill::from_percent(50);
    }

    impl pallet_reputation::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type EvidenceOrigin = EnsureRoot<Self::AccountId>;
        type JusticeOrigin = EnsureRoot<Self::AccountId>;
        type MaxVouches = ConstU32<64>;
        type VouchQuotaK = ConstU32<3>;
        type EvidenceRetention = TestRetention;
        type SponsorLiability = TestLiability;
        type EpochLength = ConstU64<10>;
    }

    fn new_test_ext() -> TestState {
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }

    #[test]
    fn recompute_writes_snapshot_and_advances_epoch() {
        new_test_ext().execute_with(|| {
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, Suit::Commerce, 5));
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, Suit::Commerce, 5));
            Reputation::run_epoch();
            assert_eq!(Epoch::<Test>::get(), 1);
            // members who put in evidence earn standing
            assert!(ReputationSnapshot::<Test>::get(1u64, Suit::Commerce) > 0);
            assert!(ReputationSnapshot::<Test>::get(2u64, Suit::Commerce) > 0);
            // an account that contributed nothing has zero standing (reputation is EARNED)
            assert_eq!(ReputationSnapshot::<Test>::get(99u64, Suit::Commerce), 0);
        });
    }

    /// Give `who` enough evidence in every suit, then recompute, so it is reputable in all four and
    /// has a positive vouch quota. Seeds 200 so that after the one epoch's decay (ρ = 0.5 in tests) the
    /// standing settles at 100 — the round number the cascade/fraud assertions below are written against.
    fn make_reputable(who: u64) {
        for suit in Suit::ALL {
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), who, suit, 200));
        }
        Reputation::run_epoch();
    }

    #[test]
    fn vouch_needs_earned_reputation_quota() {
        new_test_ext().execute_with(|| {
            // no reputation yet -> quota 0 -> cannot vouch (§1.5c)
            assert_noop!(
                Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1),
                Error::<Test>::VouchQuotaReached
            );
            make_reputable(1);
            assert!(Reputation::vouch_quota_of(&1) > 0);
            // now the earned standing buys the right to sponsor
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
        });
    }

    #[test]
    fn evidence_requires_privileged_origin() {
        new_test_ext().execute_with(|| {
            // evidence is not self-asserted: a plain signed origin cannot record it
            assert_noop!(
                Reputation::submit_evidence(RuntimeOrigin::signed(1), 1u64, Suit::Commerce, 5),
                DispatchError::BadOrigin
            );
        });
    }

    #[test]
    fn nobody_vouches_for_themselves() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                Reputation::vouch(RuntimeOrigin::signed(1), 1u64, Suit::Commerce, 1),
                Error::<Test>::SelfVouch
            );
        });
    }

    #[test]
    fn fraud_slashes_culprit_and_cascades_to_sponsor() {
        new_test_ext().execute_with(|| {
            // sponsor 1 must be reputable in all suits to vouch; it holds 100 evidence in Commerce.
            make_reputable(1);
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, Suit::Commerce, 100));
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
            // 2 defrauds: loses 100, the hit climbs ½ per hop up to depth 2
            assert_ok!(Reputation::report_fraud(RuntimeOrigin::root(), 2u64, Suit::Commerce, 100, 2));
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Commerce), 0); // culprit wiped
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 50); // sponsor answers for ½
        });
    }

    #[test]
    fn epoch_telemetry_tracks_active_members() {
        new_test_ext().execute_with(|| {
            // 1 is reputable in all four suits -> active
            make_reputable(1);
            let r = LastReport::<Test>::get().unwrap();
            assert_eq!(r.members, 1);
            assert_eq!(r.active_members, 1);
            assert!(r.top_reputation > 0);
            // 2 has only one suit -> conservative min across suits is 0 -> a member, but not active
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, Suit::Commerce, 5));
            Reputation::run_epoch();
            let r2 = LastReport::<Test>::get().unwrap();
            assert_eq!(r2.members, 2);
            assert_eq!(r2.active_members, 1);
        });
    }

    #[test]
    fn cascade_slash_does_not_double_in_a_cycle() {
        new_test_ext().execute_with(|| {
            // 1 and 2 reputable in all suits (100 evidence each) so both can vouch; mutual cycle.
            make_reputable(1);
            make_reputable(2);
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(2), 1u64, Suit::Commerce, 1));
            // 1 defrauds; the cascade must not loop back and slash 1 (or 2) twice.
            assert_ok!(Reputation::report_fraud(RuntimeOrigin::root(), 1u64, Suit::Commerce, 100, 3));
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 0); // culprit, once
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Commerce), 50); // sponsor loses ½, once (cycle guarded)
        });
    }

    #[test]
    fn genesis_seeds_the_founding_cohort() {
        // the chain boots with the founding cohort's evidence already in place (§1.4)
        let mut ext: TestState = RuntimeGenesisConfig {
            system: Default::default(),
            reputation: pallet_reputation::GenesisConfig {
                evidence: vec![(1u64, Suit::Commerce, 5), (2u64, Suit::Governance, 3)],
            },
        }
        .build_storage()
        .unwrap()
        .into();
        ext.execute_with(|| {
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 5);
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Governance), 3);
            // and the founders earn reputation on the first epoch recompute
            Reputation::run_epoch();
            assert!(ReputationSnapshot::<Test>::get(1u64, Suit::Commerce) > 0);
        });
    }

    #[test]
    fn evidence_decays_each_epoch_and_clears() {
        new_test_ext().execute_with(|| {
            // ρ = 0.5 in tests → evidence halves every epoch (§1.7 leaky integrator, Art. VI).
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, Suit::Commerce, 100));
            // genesis recompute is NOT triggered here; the store holds the raw evidence.
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 100);
            Reputation::run_epoch(); // age once
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 50);
            Reputation::run_epoch(); // age twice
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 25);
            // run it down: floor(ρ·E) reaches 0 and the entry is removed (no storage bloat).
            for _ in 0..10 {
                Reputation::run_epoch();
            }
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 0);
            assert!(!Evidence::<Test>::contains_key(1u64, Suit::Commerce));
        });
    }

    #[test]
    fn retired_standing_decays_while_active_holds() {
        new_test_ext().execute_with(|| {
            // Two members reputable in all four suits. Account 1 keeps contributing every epoch (active);
            // account 2 contributes once and then stops (retired). Anti-entrenchment (Art. VI): the
            // retiree's standing must fade while the active one holds.
            for suit in Suit::ALL {
                assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, suit, 100));
                assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, suit, 100));
            }
            // run several epochs; account 1 tops up to the universal rate each epoch, account 2 does not.
            for _ in 0..6 {
                for suit in Suit::ALL {
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, suit, 100));
                }
                Reputation::run_epoch();
            }
            // the retiree's evidence has leaked away; the active member's is sustained.
            assert!(Evidence::<Test>::get(2u64, Suit::Commerce) < Evidence::<Test>::get(1u64, Suit::Commerce));
            // and that shows up in the derived reputation the consensus reads.
            let r_active = ReputationSnapshot::<Test>::get(1u64, Suit::Commerce);
            let r_retired = ReputationSnapshot::<Test>::get(2u64, Suit::Commerce);
            assert!(r_active > r_retired, "active {r_active} should outrank retired {r_retired}");
        });
    }

    #[test]
    fn is_related_tracks_direct_vouch_either_direction() {
        new_test_ext().execute_with(|| {
            make_reputable(1); // 1 earns the standing to vouch
            assert!(Reputation::is_related(&1, &1)); // an account is "related" to itself (a party)
            assert!(!Reputation::is_related(&1, &2)); // no tie yet
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
            assert!(Reputation::is_related(&1, &2)); // forward edge 1→2
            assert!(Reputation::is_related(&2, &1)); // reverse direction also counts (Art. IX interest)
            assert!(!Reputation::is_related(&1, &3)); // an unrelated account
        });
    }

    #[test]
    fn relation_depth_sees_two_hop_ring_that_evades_depth1() {
        new_test_ext().execute_with(|| {
            make_reputable(1);
            make_reputable(2);
            // chain 1 → 2 → 3 (sponsor edges). Undirected distance: d(1,2)=1, d(1,3)=2.
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(2), 3u64, Suit::Commerce, 1));
            assert_eq!(Reputation::relation_depth(&1, &[2], 512), Some(1)); // direct tie
            assert_eq!(Reputation::relation_depth(&1, &[3], 512), Some(2)); // the 2-hop ring
            assert_eq!(Reputation::relation_depth(&3, &[1], 512), Some(2)); // undirected
            assert_eq!(Reputation::relation_depth(&1, &[9], 512), None); // independent
        });
    }

    #[test]
    fn relation_depth_catches_common_sponsor() {
        new_test_ext().execute_with(|| {
            make_reputable(1);
            // 1 sponsors BOTH 2 and 3. 2 and 3 have no direct edge (depth-1 says independent)…
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 3u64, Suit::Commerce, 1));
            assert!(!Reputation::is_related(&2, &3));
            // …but they share sponsor 1 → depth-2 (the in-edge / common-sponsor path the BFS must catch).
            assert_eq!(Reputation::relation_depth(&2, &[3], 512), Some(2));
        });
    }

    #[test]
    fn relation_depth_is_bounded_by_visit_cap() {
        new_test_ext().execute_with(|| {
            make_reputable(1);
            make_reputable(2);
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(2), 3u64, Suit::Commerce, 1));
            // depth-1 is found regardless of the cap (checked before spending budget).
            assert_eq!(Reputation::relation_depth(&1, &[2], 0), Some(1));
            // depth-2 search with a zero budget bails out → conservative None (anti-DoS, never a false tie).
            assert_eq!(Reputation::relation_depth(&1, &[3], 0), None);
        });
    }

    #[test]
    fn slash_for_verdict_cascades_like_report_fraud() {
        new_test_ext().execute_with(|| {
            make_reputable(1); // sponsor: 100 in every suit after one epoch's decay
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, Suit::Commerce, 100));
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
            // the justice pallet's verdict entry: same cascade as report_fraud, no origin needed.
            Reputation::slash_for_verdict(&2, Suit::Commerce, 100, 2);
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Commerce), 0); // culprit wiped
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 50); // sponsor answers for the share
        });
    }

    #[test]
    fn epoch_advances_automatically_with_no_root_trigger() {
        new_test_ext().execute_with(|| {
            // there is no `advance_epoch` extrinsic any more — the recompute is the chain's own clock.
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, Suit::Commerce, 100));
            // a non-boundary block does nothing (EpochLength = 10 in tests).
            Reputation::on_initialize(5u64);
            assert_eq!(Epoch::<Test>::get(), 0);
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 100);
            // the epoch boundary advances by ITSELF — no origin, no king.
            Reputation::on_initialize(10u64);
            assert_eq!(Epoch::<Test>::get(), 1);
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 50); // aged once (ρ = 0.5)
            // and again at the next boundary.
            Reputation::on_initialize(20u64);
            assert_eq!(Epoch::<Test>::get(), 2);
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 25);
        });
    }
}
