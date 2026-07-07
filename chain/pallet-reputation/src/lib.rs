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
/// Re-exported so the runtime can express reputation-scaled constants (e.g. the 🔴#5 attester floor)
/// without depending on `reputation-core` directly.
pub use reputation_core::FP_SCALE;

/// The measured-service feed (F2 piece 6, the reviewer's spec): who served how much in a JUST-ENDED
/// epoch (blocks authored in turn + finality votes in committee, counted by pallet-participation from
/// verified inherents). Consumed by `run_epoch` at the SAME cadence as everything else reputation does —
/// service becomes objective EVIDENCE (suit Technical) epoch by epoch, so a new node earns standing in
/// DAYS. This is the cold-start pump: the first evidence in a rep-zero genesis comes from running nodes,
/// not from anyone's attestation. The runtime wires it to pallet-participation; `` = no feed (tests).
pub trait ServiceFeed<AccountId> {
    fn epoch_service(epoch: u64) -> alloc::vec::Vec<(AccountId, u64)>;
}
impl<AccountId> ServiceFeed<AccountId> for () {
    fn epoch_service(_epoch: u64) -> alloc::vec::Vec<(AccountId, u64)> {
        alloc::vec::Vec::new()
    }
}

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
    use super::{EpochReport, ServiceFeed, Suit};
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

        /// (🔴#5-i, SPEC-RELAUNCH §7) Max attestations one attester may cast per epoch, shared across
        /// suits and beneficiaries (PARÁMETRO, default 8). The first of the four anti-pair-Sybil layers:
        /// however cheap a mask is, its testimony throughput is rate-limited by the chain's own clock.
        #[pallet::constant]
        type AttestBudget: Get<u32>;

        /// (🔴#5-iii, SPEC-RELAUNCH §7) ABSOLUTE floor of attester consensus reputation, raw fixed-point
        /// (PARÁMETRO). The epoch floor is `max(p25 of the rep>0 set, this)` — see [`AttestFloorFp`] —
        /// so even at genesis/empty state a zero-reputation mask's testimony admits nothing. Below the
        /// floor an attest is ACCEPTED but weighs 0 (no censorship; it just does not count).
        #[pallet::constant]
        type AttestFloorMinFp: Get<i128>;

        /// (🔴#5-v, SPEC v4 amendment — the reviewer's gate F1b) Magnitude cap multiplier `c`:
        /// one attest admits at most `c × the attester's OWN evidence in the same suit` — **nobody
        /// vouches more weight than they themselves carry**. The other layers bound testimony
        /// THROUGHPUT; this bounds its SIZE, so one fat attest from a barely-above-floor account
        /// cannot mint arbitrary evidence. PARÁMETRO, default 1.
        #[pallet::constant]
        type AttestStakeMultiplier: Get<u32>;

        /// (F2 piece 6) The measured-service feed — the runtime wires it to pallet-participation's
        /// per-epoch counters; `` disables it (unit tests).
        type ServiceFeed: super::ServiceFeed<Self::AccountId>;

        /// (F2 piece 6) Evidence units credited per unit of measured service (PARÁMETRO — reviewed
        /// jointly in F3: this dial decides how fast running a node buys standing). 0 disables.
        #[pallet::constant]
        type ServiceEvidenceRate: Get<u128>;
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

    /// Block at which the privileged bootstrap `EvidenceOrigin` stops being able to admit evidence
    /// (§1.3b). `0` = unset. Set ONCE during bootstrap; from `end` onward the founder-gated path closes
    /// **automatically, by block, with nobody's discretion** — so no permanent privileged class decides
    /// what counts as evidence (Art. V/VI/XII). After it, admission must be objective/governance (the
    /// rule-based path, next increment). This auto-expiry is the constitutionality condition of mainnet.
    #[pallet::storage]
    pub type BootstrapWindowEnd<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    /// Pending counterparty attestations (§1.3a, EO-1/EO-2): the OBJECTIVE, non-privileged evidence path.
    /// `(attester, beneficiary, suit) -> amount`. A real counterparty stakes its own name attesting a fact
    /// (a settled deal, a completed job); the beneficiary then claims it. Nobody self-asserts, and NO
    /// privileged origin admits it — admissibility is the recomputable rule "a distinct counterparty
    /// attested", which survives the bootstrap window's closure (§1.3b). This is how evidence keeps flowing
    /// once the founder-gated path expires, with no permanent privileged class. The amount stored here is
    /// the ADMITTED weight, already passed through the 🔴#5 layers (budget, pair decay, attester floor).
    #[pallet::storage]
    pub type Attestations<T: Config> =
        StorageMap<_, Blake2_128Concat, (T::AccountId, T::AccountId, Suit), u128, OptionQuery>;

    /// (🔴#5-i) Per-epoch attest budget tracking: `attester -> (epoch_of_last_use, attests_used)`. Keyed
    /// by epoch INSIDE the value so `run_epoch` never sweeps this map — a stale epoch simply reads as a
    /// fresh budget on the next attest (O(1) rollover, no unbounded clear on the epoch boundary). One
    /// small fixed-size entry per account that ever attested.
    #[pallet::storage]
    pub type AttestsThisEpoch<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, (u64, u32), ValueQuery>;

    /// (🔴#5-ii) LIFETIME saturating attest counter per `(attester, beneficiary, suit)` — the geometric
    /// r=½ pair decay: the n-th attest of a pair admits `amount >> n`, so one pair can never fabricate
    /// more than 2× a single full-weight attest per suit IN ITS WHOLE LIFE. m2 (SPEC-RELAUNCH §7,
    /// decisión consciente): this is grow-only storage — accepted trade-off for the lifetime anti-Sybil
    /// bound; each entry is one saturating u8 (counting stops at [`ATTEST_PAIR_DECAY_MAX`], past which
    /// the admitted weight is ~0 anyway) and exists only for pairs that actually transacted, so the
    /// attacker pays for the storage it creates.
    #[pallet::storage]
    pub type PairAttestCount<T: Config> =
        StorageMap<_, Blake2_128Concat, (T::AccountId, T::AccountId, Suit), u8, ValueQuery>;

    /// (🔴#5-iii) This epoch's attester floor: the 25th percentile of consensus reputation over the
    /// rep>0 set, recomputed each `run_epoch` from the SAME feed the committee is weighted on. The
    /// effective floor at attest time is `max(this, T::AttestFloorMinFp)` — the absolute minimum covers
    /// genesis/e0 where this is still 0. An attester below the floor is heard but admits 0 evidence:
    /// testimony weighs what the witness's own standing weighs (§1.3a; no censorship, no count).
    #[pallet::storage]
    pub type AttestFloorFp<T> = StorageValue<_, i128, ValueQuery>;

    /// (Acta J2, Art. IX) Next id of the evidence JOURNAL.
    #[pallet::storage]
    pub type NextEvidenceRecordId<T> = StorageValue<_, u64, ValueQuery>;

    /// (Acta J2 §3) Circular cursor of the journal dust sweep: the next record id `run_epoch` will
    /// inspect. Advances [`EVIDENCE_SWEEP_PER_EPOCH`] ids per epoch and wraps at the journal head.
    #[pallet::storage]
    pub type EvidenceSweepCursor<T> = StorageValue<_, u64, ValueQuery>;

    /// (F2 piece 6) Service-credit cursor: the NEXT service epoch to consume from the feed. Epochs
    /// strictly below it are already credited — exactly-once, whatever the boundary jitter.
    #[pallet::storage]
    pub type ServiceCreditCursor<T> = StorageValue<_, u64, ValueQuery>;

    /// (Acta J2, Art. IX — "reputation is TESTIMONY, a verdict RECTIFIES the record") The evidence
    /// JOURNAL: every admission — bootstrap `submit_evidence`, claimed attestation, genesis seed —
    /// writes an identifiable record `id -> (who, suit, amount_admitted, epoch_admitted)`. A justice
    /// case names the CONCRETE record ids it disputes; a guilty verdict strikes exactly those records
    /// and nothing else (the MECHANICAL cap: loss ≤ what the named records are worth today). Without
    /// this identity, "rectification" would degrade into a discretionary fine — the exact thing the
    /// acta forbids. A record's CURRENT worth is `amount · ρ^(epochs elapsed)` (the same retention the
    /// aggregate store decays by; integer fixed-point, deterministic). Struck (rectified) records are
    /// REMOVED — "tachar la falsedad probada"; fully-decayed ones are worth 0 and inert.
    #[pallet::storage]
    pub type EvidenceRecords<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, (T::AccountId, Suit, u128, u64), OptionQuery>;

    /// Block at which each vouch edge `(voucher, target, suit)` was cast (§1.5d probation). Lets the
    /// cascade tell a freshly-cast vouch from a matured one.
    #[pallet::storage]
    pub type VouchedAt<T: Config> =
        StorageMap<_, Blake2_128Concat, (T::AccountId, T::AccountId, Suit), BlockNumberFor<T>, OptionQuery>;

    /// Probation window (§1.5d, C4): a fraud by a protégé whose sponsor's vouch is YOUNGER than this many
    /// blocks does NOT cascade to that sponsor — the propagated trust has not matured, so there is nothing
    /// to claw back (kills the "vouch-bomb": vouch then immediately defraud to drain the sponsor). `0` = off
    /// (default), so it is opt-in and never changes existing behaviour until set. A tunable dial.
    #[pallet::storage]
    pub type ProbationWindow<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    /// §1.4b entrenchment-guard halt threshold: the largest single-entity share of consensus reputation
    /// above which an epoch counts toward the halt (≈1/3, FP_SCALE-scaled). **HARD constant — deliberately
    /// NOT a `Config` type and NOT governance-mutable**: a governance-tunable bar would let a majority cabal
    /// raise it to entrench *legally* (Art XII — no power alters the guarantees). Sealed with the code.
    pub const ENTRENCHMENT_THRESHOLD_FP: i128 = reputation_core::FP_SCALE / 3;
    /// §1.4b entrenchment-guard run length: consecutive over-threshold epochs required to HALT finality
    /// (7 ≈ one week at a daily epoch). Hard constant for the same constitutional reason as the threshold.
    pub const ENTRENCHMENT_REQUIRED_EPOCHS: u32 = 7;

    /// §1.2c λ of the concave consensus aggregate `(1−λ)·min + λ·median` (provisional 0.5) — the ONE
    /// definition shared by the full-set feed ([`Pallet::consensus_reputation`]) and the single-account
    /// read ([`Pallet::consensus_reputation_of`]), so guard, sortition and the 🔴#5 attester floor can
    /// never disagree about what a member's consensus weight is.
    pub const CONSENSUS_LAMBDA_FP: i128 = reputation_core::FP_SCALE / 2;

    /// (🔴#5-ii) Depth at which the pair attest counter saturates: past 16 halvings the pair is SPENT
    /// and admits exactly 0 — this keeps the m2 grow-only entry a single small u8 forever.
    pub const ATTEST_PAIR_DECAY_MAX: u8 = 16;

    /// (Acta J2 §3) Evidence-journal dust sweep width: how many record ids the epoch boundary scans
    /// (circular cursor) pruning records whose current worth decayed to 0. Bounds the per-epoch cost to
    /// O(64) reads however large the journal grows — deterministic, part of consensus.
    pub const EVIDENCE_SWEEP_PER_EPOCH: u64 = 64;

    /// §1.4b cold-start **entrenchment guard** — running count of consecutive epochs in which the largest
    /// single-entity share of consensus reputation has exceeded [`ENTRENCHMENT_THRESHOLD_FP`]. A share at or
    /// below the threshold **resets** it to 0 (self-healing): the guard fires only on *sustained*
    /// entrenchment, never on a transient small-N spike. Written each epoch in `run_epoch`.
    #[pallet::storage]
    pub type EntrenchmentCounter<T> = StorageValue<_, u32, ValueQuery>;

    /// §1.4b entrenchment guard **halt flag** (`counter ≥ ENTRENCHMENT_REQUIRED_EPOCHS`), written each epoch
    /// so `node/finality.rs` can read it cheaply cross-pallet (like the beacon) without recomputing the
    /// share. `true` → the finality committee is seated **EMPTY** → finality FREEZES until the share dilutes
    /// back below threshold, at which point it self-resumes. Block production is unaffected; nothing is
    /// seized or expelled (Art II property inviolable, Art IX no force). The chain merely stops *blessing*
    /// state while power is entrenched (Art VI: no perpetual authority).
    #[pallet::storage]
    pub type EntrenchmentHalted<T> = StorageValue<_, bool, ValueQuery>;

    /// §1.4b RECOVERY BANDS (self-heal fix, iron test): the history of entrenchment-halt
    /// windows as `(halt_start, cleared_at)` pairs. `halt_start` = block where the freeze armed
    /// (`halted` false→true); `cleared_at` = block where it lifted (share back < 1/3, true→false), or
    /// `None` while still frozen (the open band). The node's finality recovery checkpoint, for a stuck
    /// height h, finds the band whose `[halt_start, cleared_at]` covers h and re-anchors that height's
    /// committee to the HEALTHY reputation at `cleared_at` — so finality crosses the frozen band validated
    /// by a sane committee. A LIST (not a scalar) so a second entrenchment cycle cannot erase the first
    /// band's anchor (multi-band correctness — the guard is the constitutional backstop, it ships whole).
    /// Bounded (oldest fully-recovered bands pruned): a real chain sees ~zero halts; 8 is generous.
    /// Deterministic consensus state, written in `run_epoch`.
    #[pallet::storage]
    pub type EntrenchmentBands<T: Config> = StorageValue<
        _,
        BoundedVec<(BlockNumberFor<T>, Option<BlockNumberFor<T>>), ConstU32<8>>,
        ValueQuery,
    >;

    /// Last epoch's network telemetry (SPEC §5c) — what the nodes serve to the public panel.
    #[pallet::storage]
    pub type LastReport<T> = StorageValue<_, EpochReport, OptionQuery>;

    /// Finality-vote delegation: a committee account → the sr25519 PUBLIC key of the hot **session
    /// key** that signs Woven-Trust finality votes on its behalf. The sovereign account key stays
    /// **cold** (an offline vault; never signs); the session key is hot + disposable on the voting node. This
    /// is the authority indirection that keeps committee membership/weight keyed by the cold account
    /// while a rotatable hot key does the per-block signing. Seeded at genesis (the map IS the
    /// authorization — no `setKeys` and so no signature from the cold key is ever needed to launch).
    /// Absent → the account signs with its own key (direct, legacy/dev path).
    #[pallet::storage]
    pub type VoteKeys<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, [u8; 32], OptionQuery>;

    /// Genesis: seed the founding cohort's objective evidence (§1.4), so the chain boots with members
    /// who already have standing to bootstrap trust. Nothing here is reputation — only the evidence
    /// inputs; reputation is still DERIVED on the first epoch recompute.
    #[pallet::genesis_config]
    #[derive(frame::prelude::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        /// `(account, suit, evidence_amount)` granted at genesis.
        pub evidence: alloc::vec::Vec<(T::AccountId, Suit, u128)>,
        /// `(account, session_vote_pubkey)`: each founding committee account → the sr25519 PUBLIC key
        /// of the hot session key that signs finality on its behalf (see [`VoteKeys`]). The sovereign
        /// (account) key stays cold; only its public delegate is recorded here. Absent → direct signing.
        pub vote_keys: alloc::vec::Vec<(T::AccountId, [u8; 32])>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            for (who, suit, amount) in &self.evidence {
                Evidence::<T>::insert(who, *suit, *amount);
                // Journal the genesis grants too (Acta J2): the founding cohort's evidence is as
                // disputable before a jury as anyone's — founders are not above rectification.
                let _ = Pallet::<T>::journal_evidence(who, *suit, *amount);
            }
            // Seed the finality-vote delegation (cold account → hot session pubkey). The map itself is
            // the authorization, so no signature from the sovereign key is needed to launch.
            //
            // HARD GUARD (genesis is irreversible): the chain must NOT be able to boot with a malformed
            // delegation. Reject — by panicking the genesis build — a duplicate account, a duplicate
            // session pubkey (two members sharing one hot key → the reverse map would silently collide),
            // or a session pubkey that equals some committee account id (an alias that could let one key
            // double-count). Belt-and-suspenders with the re-bake verification.
            let mut seen_accounts = alloc::collections::BTreeSet::<&T::AccountId>::new();
            let mut seen_pubkeys = alloc::collections::BTreeSet::<[u8; 32]>::new();
            for (who, session_pubkey) in &self.vote_keys {
                assert!(seen_accounts.insert(who), "genesis vote_keys: duplicate account in delegation");
                assert!(
                    seen_pubkeys.insert(*session_pubkey),
                    "genesis vote_keys: duplicate session pubkey (two members cannot share one hot key)"
                );
            }
            for (who, _) in &self.vote_keys {
                let aliases_a_pubkey = who.using_encoded(|b| {
                    b.len() == 32 && {
                        let mut a = [0u8; 32];
                        a.copy_from_slice(b);
                        seen_pubkeys.contains(&a)
                    }
                });
                assert!(
                    !aliases_a_pubkey,
                    "genesis vote_keys: a session pubkey collides with a committee account id"
                );
            }
            for (who, session_pubkey) in &self.vote_keys {
                VoteKeys::<T>::insert(who, *session_pubkey);
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
        /// Validated evidence was recorded for `who` in `suit` (new cumulative total). `record_id` is
        /// the journal id of this admission (Acta J2) — the handle a justice case disputes it by.
        EvidenceRecorded { who: T::AccountId, suit: Suit, total: u128, record_id: u64 },
        /// `voucher` vouched for `target` in `suit` with `weight`.
        Vouched { voucher: T::AccountId, target: T::AccountId, suit: Suit, weight: u32 },
        /// `voucher` revoked its vouch(es) for `target` in `suit`.
        VouchRevoked { voucher: T::AccountId, target: T::AccountId, suit: Suit },
        /// A new epoch began, with its network telemetry (members, active members).
        EpochAdvanced { epoch: u64, members: u32, active_members: u32 },
        /// A proven fraud slashed `culprit` in `suit`; the loss cascaded up the vouch chain.
        FraudSlashed { culprit: T::AccountId, suit: Suit, loss: u128 },
        /// The one-time bootstrap evidence window was set to end at block `end` (§1.3b). After it, the
        /// privileged `EvidenceOrigin` can no longer admit evidence.
        BootstrapWindowSet { end: BlockNumberFor<T> },
        /// `attester` attested `amount` of `suit` evidence for `beneficiary` (objective path, §1.3a).
        /// `admitted` is what actually entered the pending attestation after the 🔴#5 layers (pair
        /// decay, attester floor) — 0 means the testimony was heard but carries no evidential weight.
        AttestationMade {
            attester: T::AccountId,
            beneficiary: T::AccountId,
            suit: Suit,
            amount: u128,
            admitted: u128,
        },
        /// §1.4b entrenchment guard update (Art VI/X transparency): the largest single-entity share of
        /// consensus reputation this epoch (`share_fp`, FP_SCALE-scaled), the running `counter` of
        /// consecutive over-threshold epochs, and whether finality is currently `halted`. Emitted whenever
        /// the halt state flips or the share sits over threshold, so the freeze/heal is publicly auditable.
        EntrenchmentHalt { share_fp: i128, counter: u32, halted: bool },
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
        /// The bootstrap evidence window has already been set — it is one-time (§1.3b), no moving the
        /// goalposts.
        BootstrapWindowAlreadySet,
        /// The bootstrap evidence window has closed: the privileged `EvidenceOrigin` can no longer admit
        /// evidence (§1.3b). Admission must now be objective/governance.
        BootstrapWindowClosed,
        /// Nobody attests evidence for themselves — the objective path needs a distinct counterparty.
        SelfAttestation,
        /// No matching counterparty attestation to claim (§1.3a).
        NoAttestation,
        /// (🔴#5-i) The caller has spent its attestation budget for this epoch — testimony is
        /// rate-limited by the chain's clock, however many masks are minted.
        AttestBudgetExhausted,
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
            // §1.3b: once the one-time bootstrap window has closed, the privileged origin can no longer
            // admit evidence — the founder-gated path expires automatically by block. (Until the
            // objective/governance admission path lands in the next increment, an unset window — 0 —
            // leaves behaviour unchanged, so this is a no-regression addition.)
            let end = BootstrapWindowEnd::<T>::get();
            if !end.is_zero() {
                ensure!(
                    frame_system::Pallet::<T>::block_number() < end,
                    Error::<T>::BootstrapWindowClosed
                );
            }
            let total = Evidence::<T>::mutate(&who, suit, |e| {
                *e = e.saturating_add(amount);
                *e
            });
            let record_id = Self::journal_evidence(&who, suit, amount);
            Self::deposit_event(Event::EvidenceRecorded { who, suit, total, record_id });
            Ok(())
        }

        /// Set the one-time bootstrap evidence window end (§1.3b). From block `end` onward the privileged
        /// `EvidenceOrigin` can no longer `submit_evidence`; admission must then be objective/governance.
        /// Settable ONCE (no moving the goalposts) by the same trusted `EvidenceOrigin`. This is how the
        /// founder-gated evidence path is made to **expire automatically**, satisfying the mainnet
        /// constitutionality condition that no permanent privileged class decides what counts as evidence.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn set_bootstrap_window(origin: OriginFor<T>, end: BlockNumberFor<T>) -> DispatchResult {
            T::EvidenceOrigin::ensure_origin(origin)?;
            ensure!(
                BootstrapWindowEnd::<T>::get().is_zero(),
                Error::<T>::BootstrapWindowAlreadySet
            );
            BootstrapWindowEnd::<T>::put(end);
            Self::deposit_event(Event::BootstrapWindowSet { end });
            Ok(())
        }

        /// Set the §1.5d probation window in blocks (C4). A fraud whose sponsor's vouch is younger than this
        /// does not cascade to that sponsor (anti vouch-bomb). `0` disables it. A governance dial, gated by
        /// `EvidenceOrigin`.
        #[pallet::call_index(7)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn set_probation_window(origin: OriginFor<T>, blocks: BlockNumberFor<T>) -> DispatchResult {
            T::EvidenceOrigin::ensure_origin(origin)?;
            ProbationWindow::<T>::put(blocks);
            Ok(())
        }

        /// OBJECTIVE evidence path (§1.3a, EO-1/EO-2): a counterparty attests `amount` of `suit` evidence
        /// for `beneficiary`. ANY signed account may attest — there is NO privileged origin and no founder
        /// gate, so this path keeps working after the bootstrap window closes (§1.3b). Admissibility is the
        /// recomputable rule "a distinct counterparty staked its name", not anyone's discretion. The
        /// beneficiary then `claim_attested_evidence`. You cannot attest for yourself.
        ///
        /// Anti-pair-Sybil, 🔴#5 (SPEC-RELAUNCH §7) — all four layers, together:
        /// (i) per-attester-per-epoch budget ([`Config::AttestBudget`], shared across suits);
        /// (ii) lifetime geometric pair decay — the n-th attest of `(attester, beneficiary, suit)` admits
        ///     `amount >> n`, bounding a pair's lifetime fabrication at 2× one full attest;
        /// (iii) attester floor — testimony from a mask below `max(p25 of the rep>0 set,
        ///     AttestFloorMinFp)` is accepted but admits 0 (the witness's word weighs its own standing);
        /// (iv) cost — this call sits on the runtime's feeless-budget whitelist (🔴#1): free uses are
        ///     rationed per epoch, and past the ration it costs real HLQ.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn attest_evidence(
            origin: OriginFor<T>,
            beneficiary: T::AccountId,
            suit: Suit,
            amount: u128,
        ) -> DispatchResult {
            let attester = ensure_signed(origin)?;
            ensure!(attester != beneficiary, Error::<T>::SelfAttestation);
            // (i) epoch budget: stale epoch in the entry = fresh budget (O(1) rollover, no epoch sweep).
            let epoch = Epoch::<T>::get();
            let (last_used_epoch, used) = AttestsThisEpoch::<T>::get(&attester);
            let used = if last_used_epoch == epoch { used } else { 0 };
            ensure!(used < T::AttestBudget::get(), Error::<T>::AttestBudgetExhausted);
            AttestsThisEpoch::<T>::insert(&attester, (epoch, used.saturating_add(1)));
            // (iii) floor: the testimony of a witness without standing is heard, but weighs nothing.
            // Evaluated at ATTEST time — the weight of a testimony is the standing of the witness at the
            // moment it speaks, not whatever it later becomes.
            let floor = Self::attest_floor();
            let admitted = if Self::consensus_reputation_of(&attester) < floor {
                0
            } else {
                // (ii) lifetime pair decay: n-th attest of this exact (attester, beneficiary, suit)
                // admits amount >> n; AT the saturation depth the pair is spent and admits 0 outright
                // (review: `amount >> 16` alone would admit ~amount/65k FOREVER once the
                // counter froze — an unbounded drip that breaks the 2× lifetime invariant). The counter
                // only advances when weight was actually admitted.
                let pair = (attester.clone(), beneficiary.clone(), suit);
                let n = PairAttestCount::<T>::get(&pair);
                if n >= ATTEST_PAIR_DECAY_MAX {
                    0
                } else {
                    // (v) magnitude cap (SPEC v4 amendment, gate F1b): NOBODY VOUCHES MORE WEIGHT THAN
                    // THEY THEMSELVES HOLD — admitted ≤ c × the attester's own evidence in this suit.
                    // Same units by construction (evidence caps evidence, suit for suit): the witness's
                    // word in commerce weighs at most its own commerce record.
                    let stake_cap = Evidence::<T>::get(&attester, suit)
                        .saturating_mul(u128::from(T::AttestStakeMultiplier::get()));
                    // R1 fix (audit): the shift MUST be applied to the already-capped value,
                    // NOT the raw amount. `(amount >> n).min(cap)` lets an attacker-chosen `amount ≫ cap`
                    // keep `amount >> n` above `cap` for every n∈[0,15], so `min` picks `cap` on all 16 uses
                    // → lifetime fabrication = 16×cap, defeating the "≤2× one attest" invariant (layers ii and
                    // v cancelled instead of composing). With `min(amount,cap) >> n` the geometric decay
                    // always engages: lifetime = Σ base>>n ≤ 2·min(amount,cap) ≤ 2·cap. Honest use (amount ≤
                    // cap) is byte-identical to before. Kills the reciprocal-pair 2^16 inflation attack.
                    let admitted = amount.min(stake_cap) >> u32::from(n);
                    if admitted > 0 {
                        PairAttestCount::<T>::insert(&pair, n.saturating_add(1));
                    }
                    admitted
                }
            };
            if admitted > 0 {
                Attestations::<T>::mutate((attester.clone(), beneficiary.clone(), suit), |a| {
                    *a = Some(a.unwrap_or(0).saturating_add(admitted));
                });
            }
            Self::deposit_event(Event::AttestationMade {
                attester,
                beneficiary,
                suit,
                amount,
                admitted,
            });
            Ok(())
        }

        /// Claim a counterparty's attestation as recorded evidence (§1.3a). The caller is the beneficiary;
        /// a pending attestation from `attester` for the caller in `suit` is consumed and becomes objective
        /// evidence. No privileged origin involved — this is the post-bootstrap, non-discretionary admission.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn claim_attested_evidence(
            origin: OriginFor<T>,
            attester: T::AccountId,
            suit: Suit,
        ) -> DispatchResult {
            let beneficiary = ensure_signed(origin)?;
            let amount = Attestations::<T>::take((attester, beneficiary.clone(), suit))
                .ok_or(Error::<T>::NoAttestation)?;
            let total = Evidence::<T>::mutate(&beneficiary, suit, |e| {
                *e = e.saturating_add(amount);
                *e
            });
            // Journal the admission (Acta J2). `amount` here is already the ADMITTED weight — the 🔴#5
            // layers (pair decay, attester floor) were applied when the attestation was made — so the
            // record is worth exactly what this evidence contributed, never the gross claim.
            let record_id = Self::journal_evidence(&beneficiary, suit, amount);
            Self::deposit_event(Event::EvidenceRecorded { who: beneficiary, suit, total, record_id });
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
            // §1.5d probation: stamp when this vouch was cast, so the cascade can shield a sponsor whose
            // vouch has not yet matured (anti vouch-bomb).
            VouchedAt::<T>::insert(
                (voucher.clone(), target.clone(), suit),
                frame_system::Pallet::<T>::block_number(),
            );
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
        /// All finality-vote delegations seeded at genesis: `(committee account, hot session pubkey)`.
        /// Exposed to the node through `HarlequinConsensusApi` so every node resolves a vote's signer
        /// (session key) → its committee account IDENTICALLY from on-chain state — never local config.
        pub fn vote_keys() -> alloc::vec::Vec<(T::AccountId, [u8; 32])> {
            VoteKeys::<T>::iter().collect()
        }

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
            // §1.2c (C4): the cross-suit consensus weight is now a CONCAVE aggregate (1−λ)·min + λ·median
            // with λ = 0.5 (provisional), not a hard MIN. It blunts the "MIN as a weapon" (paper §8.5) — a
            // single suit driven to 0 no longer collapses a *balanced* member's whole weight — while keeping
            // anti-specialization: a specialist/collusion vector is imbalanced (median ≈ 0) so it stays ≈ 0,
            // exactly as the MIN did (re-validated analytically + by sim, SPEC §1.2c). λ is provisional and
            // belongs in a Config dial later; the attack suite (ledger #1) is re-run on this wiring in iron.
            let mut out = alloc::vec::Vec::new();
            for acc in accounts {
                let agg = Self::consensus_reputation_of(&acc);
                if agg > 0 {
                    out.push((acc, agg));
                }
            }
            out
        }

        /// One account's consensus reputation — the same §1.2c concave aggregate over the snapshot that
        /// [`consensus_reputation`](Self::consensus_reputation) computes for the full set (single
        /// definition via [`CONSENSUS_LAMBDA_FP`]), at 4 storage reads. Used by the 🔴#5 attester floor.
        pub fn consensus_reputation_of(who: &T::AccountId) -> i128 {
            let mut suits = [0i128; 4];
            for (i, suit) in Suit::ALL.iter().enumerate() {
                suits[i] = ReputationSnapshot::<T>::get(who, *suit);
            }
            reputation_core::concave_aggregate_fp(&suits, CONSENSUS_LAMBDA_FP)
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
            // §1.5d per-incident cap (C4, anti-kamikaze): the culprit answers in full, but a SPONSOR loses
            // at most this fraction of its CURRENT standing in the suit per incident — so a single verdict
            // can never wipe a sponsor (a sleeper's deliberate "kamikaze" fraud cannot drain those who
            // vouched for it beyond a bounded slice). Several independent incidents still erode standing (a
            // real pattern of bad judgement). The fraction is a tunable dial (PARÁMETRO §1.5d).
            let cap = Permill::from_percent(50);
            let now = frame_system::Pallet::<T>::block_number();
            let probation = ProbationWindow::<T>::get();
            let mut visited: BTreeSet<T::AccountId> = BTreeSet::new();
            let mut level: Vec<(T::AccountId, u128)> = alloc::vec![(agent.clone(), amount)];
            let mut remaining = depth;
            let mut is_culprit = true;
            loop {
                let mut next: Vec<(T::AccountId, u128)> = Vec::new();
                for (who, amt) in level.drain(..) {
                    if amt == 0 || visited.contains(&who) {
                        continue;
                    }
                    visited.insert(who.clone());
                    // The culprit takes the full hit; a sponsor's loss is capped per incident (§1.5d).
                    let hit = if is_culprit {
                        amt
                    } else {
                        amt.min(cap.mul_floor(Evidence::<T>::get(&who, suit)))
                    };
                    if hit == 0 {
                        continue;
                    }
                    Evidence::<T>::mutate(&who, suit, |e| *e = e.saturating_sub(hit));
                    if remaining == 0 {
                        continue;
                    }
                    let passes_on = T::SponsorLiability::get().mul_floor(hit); // share of the ACTUAL hit
                    if passes_on == 0 {
                        continue;
                    }
                    for (voucher, edges) in Vouches::<T>::iter() {
                        if !visited.contains(&voucher)
                            && edges.iter().any(|(t, s, _)| t == &who && *s == suit)
                        {
                            // §1.5d probation: a vouch younger than the window has not matured → no clawback
                            // to that sponsor yet. Window 0 (default) = off, so behaviour is unchanged.
                            let fresh = !probation.is_zero()
                                && VouchedAt::<T>::get((&voucher, &who, suit))
                                    .map(|at| now.saturating_sub(at) < probation)
                                    .unwrap_or(false);
                            if !fresh {
                                next.push((voucher, passes_on));
                            }
                        }
                    }
                }
                if next.is_empty() || remaining == 0 {
                    break;
                }
                level = next;
                remaining -= 1;
                is_culprit = false;
            }
        }

        /// Close one epoch: age all evidence (§1.7 leaky integrator), recompute every member's reputation
        /// from the on-chain inputs, write the snapshot + telemetry, and bump the epoch counter. Called
        /// automatically by `on_initialize` at each epoch boundary — never by an extrinsic, so no privileged
        /// actor controls the recount. Genesis is NOT affected (it calls `recompute_reputation` directly, so
        /// the founding evidence first ages on the move into epoch 1, never before standing is earned).
        pub(crate) fn run_epoch() {
            Self::decay_evidence();
            // Service → evidence AFTER the decay, BEFORE the recompute: the epoch's fresh service
            // enters this snapshot at full weight, and starts decaying only from the next epoch on.
            Self::credit_service_evidence();
            let report = Self::recompute_reputation().unwrap_or_default();
            LastReport::<T>::put(report.clone());
            // The consensus-reputation feed is computed ONCE per epoch and shared by the entrenchment
            // guard and the 🔴#5 attester floor — the two can never disagree with the committee weights.
            let reps = Self::consensus_reputation();
            Self::step_entrenchment_guard(&reps);
            Self::update_attest_floor(&reps);
            Self::sweep_dust_records();
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

        /// §1.4b entrenchment guard step (Art VI/X): after the epoch snapshot is recomputed, measure the
        /// largest single-entity share of consensus reputation and advance the sustained-over-threshold
        /// counter. `counter ≥ ENTRENCHMENT_REQUIRED_EPOCHS` sets the halt flag that `node/finality.rs` reads
        /// to seat an EMPTY committee (finality freezes; block production continues; nothing is seized). A
        /// share back at/under threshold **resets** the counter → finality self-resumes. The reputation feed
        /// is EXACTLY [`consensus_reputation`](Self::consensus_reputation) — the same values the committee is
        /// weighted on (`node/finality.rs`) — so guard and sortition can never disagree about who holds how
        /// much. v1 passes an EMPTY cluster map (`cold_start_halt_share_fp` collapses to single-entity);
        /// anti-split clustering is #754 (enabling it now would false-halt the mutually-vouching founders).
        fn step_entrenchment_guard(consensus_reps: &[(T::AccountId, i128)]) {
            use alloc::string::ToString;
            // v1 keys reps by positional index — fine because labels are empty, so the share is key-agnostic
            // (`max_i r_i / Σ r_j` doesn't depend on the ids). #754 (cluster anti-split) MUST re-key BOTH this
            // map AND the labels map by the REAL account (SCALE-hex of `_acc`) so `communities` can join
            // account → cluster; that rewrite lives with #754, not here. Negatives are safe: the share fns
            // filter `r > 0` on numerator AND denominator, so slashed/decayed reps can't shrink Σ or inflate.
            let reps: alloc::collections::BTreeMap<alloc::string::String, i128> = consensus_reps
                .iter()
                .enumerate()
                .map(|(i, (_acc, r))| (i.to_string(), *r))
                .collect();
            // v1: empty labels → cold_start_halt_share_fp collapses to the single-entity share. Passed
            // explicitly so the call site is already cluster-ready for #754 (populate from communities).
            let labels: alloc::collections::BTreeMap<alloc::string::String, alloc::string::String> =
                alloc::collections::BTreeMap::new();
            let share_fp = reputation_core::cold_start_halt_share_fp(&labels, &reps);
            let (counter, halted) = reputation_core::entrenchment_halt_step(
                share_fp,
                ENTRENCHMENT_THRESHOLD_FP,
                EntrenchmentCounter::<T>::get(),
                ENTRENCHMENT_REQUIRED_EPOCHS,
            );
            let prev_halted = EntrenchmentHalted::<T>::get();
            EntrenchmentCounter::<T>::put(counter);
            EntrenchmentHalted::<T>::put(halted);
            // RECOVERY BANDS (self-heal fix, iron test): track each freeze window so the node's
            // finality checkpoint can re-anchor the STUCK halt-band committees to a healthy, post-dilution
            // reputation instead of the band's frozen `halted=true` state — crossing the band validated by a
            // sane committee, never finalising under a compromised one. Deterministic consensus state.
            let now = frame_system::Pallet::<T>::block_number();
            if !prev_halted && halted {
                // Freeze armed: open a new band (cleared_at = None). Prune the oldest if at the bound.
                EntrenchmentBands::<T>::mutate(|bands| {
                    if bands.is_full() {
                        bands.remove(0);
                    }
                    let _ = bands.try_push((now, None));
                });
            } else if prev_halted && !halted {
                // Freeze lifted (share guaranteed < 1/3): close the open band with the recovery height.
                EntrenchmentBands::<T>::mutate(|bands| {
                    if let Some(last) = bands.last_mut() {
                        if last.1.is_none() {
                            last.1 = Some(now);
                        }
                    }
                });
            }
            // Transparency (Art X): surface a flip of the freeze, or any epoch spent over threshold.
            if halted != prev_halted || share_fp > ENTRENCHMENT_THRESHOLD_FP {
                Self::deposit_event(Event::EntrenchmentHalt { share_fp, counter, halted });
            }
        }

        /// The EFFECTIVE attester floor right now: `max(p25 of the rep>0 set, the absolute minimum)`.
        /// The network's own bar for "high standing" — reused wherever one is needed (attest admission,
        /// multisig seat cession) so there is ONE bar, not one per pallet.
        pub fn attest_floor() -> i128 {
            AttestFloorFp::<T>::get().max(T::AttestFloorMinFp::get())
        }

        /// (🔴#5-iii) Recompute this epoch's attester floor: the 25th percentile of the rep>0 consensus
        /// set. Fed by the SAME per-epoch feed as the guard and the committee weights. With an empty
        /// positive set the stored floor is 0 and the absolute [`Config::AttestFloorMinFp`] alone gates
        /// (evaluated at attest time), so a cold network is protected from day zero.
        fn update_attest_floor(consensus_reps: &[(T::AccountId, i128)]) {
            let mut vals: alloc::vec::Vec<i128> =
                consensus_reps.iter().map(|(_, r)| *r).filter(|r| *r > 0).collect();
            let floor = if vals.is_empty() {
                0
            } else {
                vals.sort_unstable();
                vals[vals.len() / 4]
            };
            AttestFloorFp::<T>::put(floor);
        }

        /// Whether finality is currently frozen by the §1.4b entrenchment guard. Exposed to the node via the
        /// consensus runtime API; while `true` the node seats an EMPTY finality committee (finality pauses,
        /// block production continues) and self-resumes when the entrenched share dilutes back below
        /// threshold. Reads a single stored bool — cheap enough to call per block on the import path.
        pub fn entrenchment_halted() -> bool {
            EntrenchmentHalted::<T>::get()
        }

        /// §1.4b self-heal: for a finality-stuck `height`, the RECOVERY ANCHOR `H_r` if its halt band has
        /// cleared — the block whose (healthy, post-dilution) reputation the node re-anchors that height's
        /// committee to, so finality crosses the frozen band validated by a sane committee. `None` if the
        /// height is in no recovered band (never halted there, or still frozen → committee stays empty).
        /// Deterministic: reads only consensus state. Picks the band whose `[halt_start, cleared_at]`
        /// covers `height`; on the (bounded, unusual) multi-band overlap, the EARLIEST covering cleared
        /// band wins so recovery is stable and monotone.
        pub fn entrenchment_recovery_for(height: BlockNumberFor<T>) -> Option<BlockNumberFor<T>> {
            for (start, cleared) in EntrenchmentBands::<T>::get().iter() {
                if let Some(h_r) = cleared {
                    if height >= *start && height <= *h_r {
                        return Some(*h_r);
                    }
                }
            }
            None
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

        /// (F2 piece 6, the reviewer's spec) Consume the measured-service feed for every epoch
        /// not yet credited and turn it into objective EVIDENCE in the Technical suit — the cold-start
        /// pump: running a node earns standing, no attestation, no privileged origin, journaled like
        /// any other admission (Acta J2 — service evidence is as disputable as the rest). Exactly-once
        /// by cursor; the catch-up loop is bounded (normally exactly one iteration per epoch).
        fn credit_service_evidence() {
            let rate = T::ServiceEvidenceRate::get();
            let len: u64 = T::EpochLength::get().saturated_into::<u64>();
            if rate == 0 || len == 0 {
                return;
            }
            let now: u64 = frame_system::Pallet::<T>::block_number().saturated_into::<u64>();
            let ended = (now / len).saturating_sub(1);
            let mut next = ServiceCreditCursor::<T>::get();
            let mut steps = 0u32;
            while next <= ended && steps < 8 {
                for (who, weight) in T::ServiceFeed::epoch_service(next) {
                    let amount = u128::from(weight).saturating_mul(rate);
                    if amount == 0 {
                        continue;
                    }
                    let total = Evidence::<T>::mutate(&who, Suit::Technical, |e| {
                        *e = e.saturating_add(amount);
                        *e
                    });
                    let record_id = Self::journal_evidence(&who, Suit::Technical, amount);
                    Self::deposit_event(Event::EvidenceRecorded {
                        who,
                        suit: Suit::Technical,
                        total,
                        record_id,
                    });
                }
                next = next.saturating_add(1);
                steps += 1;
            }
            ServiceCreditCursor::<T>::put(next);
        }

        /// (Acta J2) Append an admission to the evidence journal and return its id. Every path that adds
        /// to `Evidence` MUST pass through here — the journal is what makes each admitted piece of
        /// evidence individually disputable before a jury (Art. IX: rectification needs identity).
        fn journal_evidence(who: &T::AccountId, suit: Suit, amount: u128) -> u64 {
            let id = NextEvidenceRecordId::<T>::mutate(|n| {
                let id = *n;
                *n = n.saturating_add(1);
                id
            });
            EvidenceRecords::<T>::insert(id, (who.clone(), suit, amount, Epoch::<T>::get()));
            id
        }

        /// (Acta J2) A journal record's CURRENT worth: `amount · ρ^(epochs elapsed)` — the same
        /// retention the aggregate evidence store decays by, in integer fixed-point (`Permill`
        /// `saturating_pow`), so what a verdict can rectify is what the falsehood is still worth today,
        /// not what it was worth when spoken. Per-record rounding may differ from the aggregate's
        /// floor-of-sums by at most a few units; `cascade_slash` subtracts saturating, so the drift can
        /// never underflow the store. `None` if the record does not exist (never journaled, already
        /// struck, or swept as dust).
        pub fn evidence_record_current(id: u64) -> Option<(T::AccountId, Suit, u128)> {
            let (who, suit, amount, epoch_admitted) = EvidenceRecords::<T>::get(id)?;
            let elapsed = Epoch::<T>::get().saturating_sub(epoch_admitted);
            let retained = T::EvidenceRetention::get()
                .saturating_pow(elapsed.min(u64::from(u32::MAX)) as usize);
            Some((who, suit, retained.mul_floor(amount)))
        }

        /// (Acta J2 §2, the reviewer's review) Whether journal record `id` is live evidence OF `who` IN `suit`
        /// with non-zero current worth. The justice pallet calls this at `open_case` so a case naming
        /// foreign, struck or dust evidence is rejected EARLY — not silently zeroed at the verdict.
        pub fn owns_evidence(id: u64, who: &T::AccountId, suit: Suit) -> bool {
            match Self::evidence_record_current(id) {
                Some((owner, s, current)) => owner == *who && s == suit && current > 0,
                None => false,
            }
        }

        /// Carry out a jury verdict's reputational consequence — **rectification of the record, not a
        /// fine** (Acta J2, Art. IX: reputation is TESTIMONY; a guilty verdict strikes a PROVEN
        /// falsehood from the registry — "verdad sabida" — it never punishes beyond it). The MECHANICAL
        /// cap, non-discretionary by construction: the applied loss is `min(requested_loss, Σ current
        /// worth of the case's own evidence records)` — a jury cannot take more than the falsehood it
        /// proved is worth today (gate E5). No valid records → nothing is applied and the culprit's
        /// standing is untouched (gate E6: the verdict then only MARKS, in the justice pallet). The
        /// named records are STRUCK (removed) — immediately and irreversibly, so no second case can
        /// rectify the same falsehood twice. The subtraction cascades up the vouch chain exactly like
        /// any slash (§1.5c/§1.7). Returns the loss actually applied.
        ///
        /// Public entry for the justice pallet's verdict handler — the proven verdict IS the authority,
        /// so no extrinsic origin is needed here (the gating lives in the justice pallet: only a quorum
        /// of a non-interested jury reaches this). `depth` bounds the cascade.
        pub fn rectify_for_verdict(
            culprit: &T::AccountId,
            suit: Suit,
            evidence_ids: &[u64],
            requested_loss: u128,
            depth: u8,
        ) -> u128 {
            use alloc::collections::BTreeSet;
            // Dedup defensively: a repeated id must not inflate the cap.
            let ids: BTreeSet<u64> = evidence_ids.iter().copied().collect();
            let mut cap: u128 = 0;
            let mut struck: alloc::vec::Vec<u64> = alloc::vec::Vec::new();
            for id in ids {
                if let Some((owner, s, current)) = Self::evidence_record_current(id) {
                    if owner == *culprit && s == suit {
                        cap = cap.saturating_add(current);
                        struck.push(id);
                    }
                }
            }
            let applied = requested_loss.min(cap);
            if applied > 0 {
                Self::cascade_slash(culprit, suit, applied, depth);
            }
            // Strike the falsehoods the verdict named — gone from the journal, unusable in any other case.
            for id in struck {
                EvidenceRecords::<T>::remove(id);
            }
            applied
        }

        /// (Acta J2 §3) Bounded dust sweep of the evidence journal, run once per epoch: scan
        /// [`EVIDENCE_SWEEP_PER_EPOCH`] ids from a circular cursor and prune records whose current
        /// worth has decayed to 0 (they can no longer anchor any rectification). Keeps the journal's
        /// live size proportional to evidence that still matters — tablet-sized state — at a
        /// deterministic O(64) cost however large the journal history grows.
        fn sweep_dust_records() {
            let head = NextEvidenceRecordId::<T>::get();
            if head == 0 {
                return;
            }
            let mut cursor = EvidenceSweepCursor::<T>::get();
            for _ in 0..EVIDENCE_SWEEP_PER_EPOCH {
                if cursor >= head {
                    cursor = 0;
                }
                if let Some((_, _, current)) = Self::evidence_record_current(cursor) {
                    if current == 0 {
                        EvidenceRecords::<T>::remove(cursor);
                    }
                }
                cursor = cursor.saturating_add(1);
            }
            EvidenceSweepCursor::<T>::put(cursor);
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
    use crate::{
        Epoch, EntrenchmentBands, EntrenchmentCounter, EntrenchmentHalted, Error, Evidence, LastReport,
        ReputationSnapshot, Suit, VoteKeys, ENTRENCHMENT_REQUIRED_EPOCHS,
    };
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
        // 🔴#5-iii absolute attester floor: 1 raw fixed-point unit in tests — any real standing passes,
        // a zero-reputation (fresh) mask does not. Mainnet uses a meaningful fraction of FP_SCALE.
        pub const TestAttestFloorMin: i128 = 1;
        // F2 piece 6: mock service feed entries `(epoch, account, weight)`.
        pub storage MockService: Vec<(u64, u64, u64)> = Vec::new();
    }

    pub struct MockServiceFeed;
    impl crate::ServiceFeed<u64> for MockServiceFeed {
        fn epoch_service(epoch: u64) -> Vec<(u64, u64)> {
            MockService::get()
                .into_iter()
                .filter(|(e, _, _)| *e == epoch)
                .map(|(_, who, w)| (who, w))
                .collect()
        }
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
        // 🔴#5-i: 3 attests/epoch in tests so budget exhaustion is cheap to exercise (mainnet: 8).
        type AttestBudget = ConstU32<3>;
        type AttestFloorMinFp = TestAttestFloorMin;
        // 🔴#5-v: c = 1 — one attest admits at most the attester's own evidence in the suit.
        type AttestStakeMultiplier = ConstU32<1>;
        type ServiceFeed = MockServiceFeed;
        type ServiceEvidenceRate = ConstU128<1>;
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
    fn bootstrap_evidence_window_expires() {
        // §1.3b / EO-3: the privileged bootstrap EvidenceOrigin admits evidence only until the one-time
        // window closes; from `end` onward it cannot, automatically by block, with nobody's discretion.
        new_test_ext().execute_with(|| {
            // open bootstrap: the privileged origin can admit
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, Suit::Commerce, 5));
            // set the one-time window to end at block 100
            assert_ok!(Reputation::set_bootstrap_window(RuntimeOrigin::root(), 100));
            // one-time: cannot move the goalposts
            assert_noop!(
                Reputation::set_bootstrap_window(RuntimeOrigin::root(), 200),
                Error::<Test>::BootstrapWindowAlreadySet
            );
            // before close, evidence still admits
            System::set_block_number(50);
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, Suit::Commerce, 5));
            // at/after close, the founder-gated path is shut — no permanent privileged evidence authority
            System::set_block_number(100);
            assert_noop!(
                Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, Suit::Commerce, 5),
                Error::<Test>::BootstrapWindowClosed
            );
        });
    }

    #[test]
    fn objective_attestation_path_records_evidence() {
        // §1.3a / EO-1/EO-2: the objective, non-privileged path. A counterparty attests; the beneficiary
        // claims; evidence is recorded WITHOUT any EvidenceOrigin/root — plain signed accounts, by rule.
        new_test_ext().execute_with(|| {
            // the attester needs standing (🔴#5-iii floor): a witness's word weighs what the witness weighs
            make_reputable(1);
            // 1 attests for 2 (a distinct counterparty staking its name) — signed, NOT root
            assert_ok!(Reputation::attest_evidence(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 7));
            // nobody attests for themselves
            assert_noop!(
                Reputation::attest_evidence(RuntimeOrigin::signed(1), 1u64, Suit::Commerce, 7),
                Error::<Test>::SelfAttestation
            );
            // beneficiary claims → evidence recorded with no privileged origin
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Commerce), 0);
            assert_ok!(Reputation::claim_attested_evidence(RuntimeOrigin::signed(2), 1u64, Suit::Commerce));
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Commerce), 7);
            // attestation consumed — no double claim
            assert_noop!(
                Reputation::claim_attested_evidence(RuntimeOrigin::signed(2), 1u64, Suit::Commerce),
                Error::<Test>::NoAttestation
            );
        });
    }

    // ───────────────────────── 🔴#5 attest anti-pair-Sybil (SPEC-RELAUNCH §7) ─────────────────────────

    #[test]
    fn attest_budget_caps_per_epoch_and_rolls_over() {
        // (i) the per-epoch budget is shared across beneficiaries/suits and refills at the epoch boundary
        // WITHOUT any storage sweep (epoch-keyed entry).
        new_test_ext().execute_with(|| {
            make_reputable(1);
            for beneficiary in 2u64..=4 {
                assert_ok!(Reputation::attest_evidence(
                    RuntimeOrigin::signed(1),
                    beneficiary,
                    Suit::Commerce,
                    10
                ));
            }
            // budget (3 in tests) spent — the 4th attest this epoch is refused
            assert_noop!(
                Reputation::attest_evidence(RuntimeOrigin::signed(1), 5u64, Suit::Commerce, 10),
                Error::<Test>::AttestBudgetExhausted
            );
            // next epoch: the stale entry reads as a fresh budget
            Reputation::run_epoch();
            assert_ok!(Reputation::attest_evidence(RuntimeOrigin::signed(1), 5u64, Suit::Commerce, 10));
        });
    }

    #[test]
    fn pair_decay_bounds_lifetime_fabrication_at_twice_one_attest() {
        // (ii) the n-th attest of one (attester, beneficiary, suit) pair admits amount >> n: geometric
        // r=½, so a pair can never fabricate more than 2× one full-weight attest per suit, EVER.
        new_test_ext().execute_with(|| {
            make_reputable(1);
            for _ in 0..3 {
                assert_ok!(Reputation::attest_evidence(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 100));
            }
            assert_ok!(Reputation::claim_attested_evidence(RuntimeOrigin::signed(2), 1u64, Suit::Commerce));
            // 100 + 50 + 25 = 175 < 2×100 — the geometric lifetime bound
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Commerce), 175);
        });
    }

    #[test]
    fn pair_decay_saturation_admits_zero_not_a_drip() {
        // R1 fix (audit): with `min(amount, cap) >> n` the geometric decay engages on the
        // CAPPED value, so an attacker-chosen `amount ≫ cap` can NO LONGER admit `cap` on every use. The
        // pre-fix order `(amount >> n).min(cap)` let `amount = cap·2^16` admit `cap` on all 16 uses =
        // ≈16×cap lifetime, the root of the reciprocal-pair 2^16 inflation vector. The corrected invariant:
        // a pair's LIFETIME fabrication is bounded at < 2×cap regardless of `amount`, and the pair spends
        // out to 0 (no drip). This test hammers with `u128::MAX` — the worst case the old code mishandled.
        new_test_ext().execute_with(|| {
            make_reputable(1); // Evidence(1,*) = 100 → cap = 100
            // Hammer B with amount ≫ cap across several epochs (budget 3/epoch); refill the attester each
            // epoch so its cap stays ≈100 and ONLY the pair decay — not a shrinking cap — bounds the total.
            for _ in 0..6 {
                for suit in Suit::ALL {
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, suit, 100));
                }
                for _ in 0..3 {
                    assert_ok!(Reputation::attest_evidence(
                        RuntimeOrigin::signed(1),
                        2u64,
                        Suit::Commerce,
                        u128::MAX
                    ));
                }
                Reputation::run_epoch();
            }
            let total = crate::Attestations::<Test>::get((1u64, 2u64, Suit::Commerce)).unwrap_or(0);
            // LIFETIME BOUND: < 2×cap. With cap ≈ 100 the geometric series 100+50+25+… settles < 200; the
            // pre-fix order bug would have admitted ≈16×cap here. Bounded generously at 600 to be robust to
            // the exact steady-state cap while still proving the ≈16× (≫600) inflation is dead.
            assert!(total < 600, "R1: pair lifetime {total} must be < 2×cap; pre-fix bug ≈ 16×cap ≫ 600");
            // the pair is SPENT (n past the decay horizon): a further u128::MAX attest admits nothing.
            let before = total;
            assert_ok!(Reputation::attest_evidence(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, u128::MAX));
            assert_eq!(
                crate::Attestations::<Test>::get((1u64, 2u64, Suit::Commerce)).unwrap_or(0),
                before,
                "spent pair admits 0, no drip"
            );
        });
    }

    #[test]
    fn reciprocal_pair_max_amount_does_not_inflate() {
        // R1 fix (audit), the ATTACK the fix exists for: two masks A(=1)/B(=2) alternate
        // max-amount attest→claim to bootstrap each other's evidence. Under the pre-fix order
        // `(amount >> n).min(cap)`, each admit = the (growing) cap on all uses → reciprocal COMPOUNDING to
        // ≈2^16× a seed. With `min(amount, cap) >> n` the geometric decay engages, so per-pair use admits
        // `cap >> n`; the shift outruns the linear cap-growth and the whole exchange stays bounded at a
        // small constant factor of the seed. All rounds run in ONE epoch so the ρ=0.5 test-decay does not
        // mask the effect. Pre-fix: A settles ≈2100+ after 3 rounds (and explodes with more); with the fix
        // ≈600.
        new_test_ext().execute_with(|| {
            for suit in Suit::ALL {
                assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, suit, 200));
                assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, suit, 200));
            }
            Reputation::run_epoch(); // both settle at Evidence = 100 in every suit; both are reputable
            let seed = Evidence::<Test>::get(1u64, Suit::Commerce);
            assert_eq!(seed, 100);
            // 3 reciprocal rounds (budget is 3/epoch/attester, so this exactly fills one epoch's budget).
            for _ in 0..3 {
                assert_ok!(Reputation::attest_evidence(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, u128::MAX));
                assert_ok!(Reputation::claim_attested_evidence(RuntimeOrigin::signed(2), 1u64, Suit::Commerce));
                assert_ok!(Reputation::attest_evidence(RuntimeOrigin::signed(2), 1u64, Suit::Commerce, u128::MAX));
                assert_ok!(Reputation::claim_attested_evidence(RuntimeOrigin::signed(1), 2u64, Suit::Commerce));
            }
            let a = Evidence::<Test>::get(1u64, Suit::Commerce);
            let b = Evidence::<Test>::get(2u64, Suit::Commerce);
            // BOUND: neither mask inflates past a small constant of the seed. 10×seed cleanly excludes the
            // pre-fix reciprocal explosion (which is already >20×seed by round 3 and grows without bound).
            assert!(a < 10 * seed, "R1: A evidence {a} must stay bounded (< 10×seed {}); pre-fix ≫", 10 * seed);
            assert!(b < 10 * seed, "R1: B evidence {b} must stay bounded (< 10×seed {}); pre-fix ≫", 10 * seed);
        });
    }

    #[test]
    fn attest_magnitude_is_capped_by_the_attesters_own_stake() {
        // (v) gate F1b (SPEC v4 amendment): an absurd-magnitude attest from an above-floor attester
        // admits at most c × the attester's OWN evidence in that suit — nobody vouches more weight
        // than they themselves carry. Throughput layers can't catch this; the size cap does.
        new_test_ext().execute_with(|| {
            make_reputable(1); // 1 holds 100 evidence per suit after the epoch's decay
            assert_ok!(Reputation::attest_evidence(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, u128::MAX));
            assert_eq!(
                crate::Attestations::<Test>::get((1u64, 2u64, Suit::Commerce)).unwrap(),
                100,
                "admitted = min(amount, 1 × attester's Commerce evidence)"
            );
            // and the cap composes with the pair decay: next attest is CAPPED then halved (R1 fix — the
            // shift is applied to the already-capped value, not the raw amount)
            assert_ok!(Reputation::attest_evidence(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 120));
            assert_eq!(
                crate::Attestations::<Test>::get((1u64, 2u64, Suit::Commerce)).unwrap(),
                150,
                "second attest admits min(120, 100) >> 1 = 50 (R1: cap BEFORE shift)"
            );
        });
    }

    #[test]
    fn attest_floor_zeroes_testimony_without_standing() {
        // (iii) an attester below the epoch floor (p25 of the rep>0 set, absolute-min covered) is heard
        // but admits 0 — no censorship, no count. The reputable counterparty's attest still counts.
        new_test_ext().execute_with(|| {
            // four equal founders anchor the p25; account 5 holds a sliver of standing (rep > 0 but
            // far below the 25th percentile)
            for who in 1u64..=4 {
                for suit in Suit::ALL {
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), who, suit, 1_000_000));
                }
            }
            for suit in Suit::ALL {
                assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 5u64, suit, 100));
            }
            Reputation::run_epoch();
            assert!(Reputation::consensus_reputation_of(&5) > 0, "5 has real (tiny) standing");
            // 5's testimony is accepted but weighs nothing
            assert_ok!(Reputation::attest_evidence(RuntimeOrigin::signed(5), 6u64, Suit::Commerce, 1_000));
            assert_noop!(
                Reputation::claim_attested_evidence(RuntimeOrigin::signed(6), 5u64, Suit::Commerce),
                Error::<Test>::NoAttestation
            );
            // a founder's testimony (at/above the floor) admits in full
            assert_ok!(Reputation::attest_evidence(RuntimeOrigin::signed(1), 6u64, Suit::Commerce, 1_000));
            assert_ok!(Reputation::claim_attested_evidence(RuntimeOrigin::signed(6), 1u64, Suit::Commerce));
            assert_eq!(Evidence::<Test>::get(6u64, Suit::Commerce), 1_000);
        });
    }

    #[test]
    fn fresh_sybil_pair_fabricates_nothing() {
        // THE F3 gate test (SPEC-RELAUNCH §7-🔴#5): two fresh masks attest to each other massively,
        // across several epochs. The four layers must leave them with ZERO admitted evidence and ZERO
        // consensus share — a Sybil pair cannot bootstrap itself into the committee.
        new_test_ext().execute_with(|| {
            for _round in 0..4 {
                for _ in 0..3 {
                    // budget-max each epoch, huge amounts
                    assert_ok!(Reputation::attest_evidence(
                        RuntimeOrigin::signed(8),
                        9u64,
                        Suit::Commerce,
                        u128::MAX / 2
                    ));
                    assert_ok!(Reputation::attest_evidence(
                        RuntimeOrigin::signed(9),
                        8u64,
                        Suit::Commerce,
                        u128::MAX / 2
                    ));
                }
                Reputation::run_epoch();
            }
            // nothing admitted: no pending attestation to claim, no evidence, no reputation, no share
            assert_noop!(
                Reputation::claim_attested_evidence(RuntimeOrigin::signed(9), 8u64, Suit::Commerce),
                Error::<Test>::NoAttestation
            );
            assert_eq!(Evidence::<Test>::get(8u64, Suit::Commerce), 0);
            assert_eq!(Evidence::<Test>::get(9u64, Suit::Commerce), 0);
            assert!(Reputation::consensus_reputation().is_empty(), "no member earned any share");
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
    fn cascade_per_incident_cap_bounds_the_kamikaze() {
        // §1.5d / C4: a sleeper (2) accrues a HUGE position then defrauds it all to drain its sponsor (1).
        // Without the cap, the cascade share (½·10000 = 5000) would wipe sponsor 1 (only 100) to 0. With the
        // per-incident cap (≤ 50% of the sponsor's standing), 1 loses at most 50 — bounded, not annihilated.
        new_test_ext().execute_with(|| {
            make_reputable(1); // sponsor 1 holds 100 in Commerce
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, Suit::Commerce, 10_000));
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
            assert_ok!(Reputation::report_fraud(RuntimeOrigin::root(), 2u64, Suit::Commerce, 10_000, 2));
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Commerce), 0); // culprit wiped (answers in full)
            // sponsor capped at 50% of its 100, NOT drained to 0 by the 5000 cascade share
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 50);
        });
    }

    #[test]
    fn probation_shields_sponsor_from_a_fresh_vouch_fraud() {
        // §1.5d / C4: with a probation window set, a protégé's fraud does NOT cascade to a sponsor whose
        // vouch is still fresh (the propagated trust has not matured) — kills the "vouch-bomb". Window 0
        // (default) leaves the existing cascade tests untouched.
        new_test_ext().execute_with(|| {
            make_reputable(1); // sponsor 1 holds 100 in Commerce
            assert_ok!(Reputation::set_probation_window(RuntimeOrigin::root(), 100));
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, Suit::Commerce, 100));
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1)); // vouched at block 0
            assert_ok!(Reputation::report_fraud(RuntimeOrigin::root(), 2u64, Suit::Commerce, 100, 2));
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Commerce), 0); // culprit answers in full
            // sponsor SHIELDED: the vouch is within the probation window, so it is not cascaded
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 100);
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
                vote_keys: vec![],
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
    fn genesis_seeds_vote_key_delegation() {
        // A founder account maps to its hot session pubkey; the cold account key seeds nothing else.
        let ext: TestState = RuntimeGenesisConfig {
            system: Default::default(),
            reputation: pallet_reputation::GenesisConfig {
                evidence: vec![(1u64, Suit::Commerce, 5)],
                vote_keys: vec![(1u64, [9u8; 32])],
            },
        }
        .build_storage()
        .unwrap()
        .into();
        let mut ext = ext;
        ext.execute_with(|| {
            assert_eq!(VoteKeys::<Test>::get(1u64), Some([9u8; 32]));
        });
    }

    #[test]
    #[should_panic(expected = "duplicate session pubkey")]
    fn genesis_rejects_duplicate_session_pubkey() {
        // Two founders cannot share one hot session key — genesis must refuse to build.
        let _ext: TestState = RuntimeGenesisConfig {
            system: Default::default(),
            reputation: pallet_reputation::GenesisConfig {
                evidence: vec![(1u64, Suit::Commerce, 5), (2u64, Suit::Commerce, 5)],
                vote_keys: vec![(1u64, [7u8; 32]), (2u64, [7u8; 32])],
            },
        }
        .build_storage()
        .unwrap()
        .into();
    }

    #[test]
    #[should_panic(expected = "duplicate account")]
    fn genesis_rejects_duplicate_account_delegation() {
        // One account cannot register two session keys.
        let _ext: TestState = RuntimeGenesisConfig {
            system: Default::default(),
            reputation: pallet_reputation::GenesisConfig {
                evidence: vec![(1u64, Suit::Commerce, 5)],
                vote_keys: vec![(1u64, [7u8; 32]), (1u64, [8u8; 32])],
            },
        }
        .build_storage()
        .unwrap()
        .into();
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
    fn rectify_for_verdict_cascades_and_is_capped_by_the_named_evidence() {
        // Acta J2: the verdict entry rectifies EXACTLY the records the case named — cascade like any
        // slash, but mechanically capped at what those records are worth (gate E5).
        new_test_ext().execute_with(|| {
            make_reputable(1); // sponsor: 100 in every suit after one epoch's decay
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, Suit::Commerce, 100));
            // the record id of 2's admission is the journal head minus 1 (just written)
            let rec = crate::NextEvidenceRecordId::<Test>::get() - 1;
            assert!(Reputation::owns_evidence(rec, &2u64, Suit::Commerce));
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
            // the jury asks for 10_000 — the mechanical cap applies min(10_000, worth of `rec`) = 100
            let applied = Reputation::rectify_for_verdict(&2, Suit::Commerce, &[rec], 10_000, 2);
            assert_eq!(applied, 100, "E5: rectification cannot exceed the case's evidence");
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Commerce), 0); // the falsehood struck
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 50); // sponsor answers for the share
            // the record is GONE — a second case cannot rectify the same falsehood again
            assert!(!Reputation::owns_evidence(rec, &2u64, Suit::Commerce));
            assert_eq!(Reputation::rectify_for_verdict(&2, Suit::Commerce, &[rec], 10_000, 2), 0);
        });
    }

    #[test]
    fn rectify_without_valid_evidence_touches_nothing() {
        // Gate E6: a verdict that names no (valid) evidence records applies ZERO loss — the standing
        // is untouched; the culprit is only marked (in the justice pallet). Foreign records are inert.
        new_test_ext().execute_with(|| {
            make_reputable(1);
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, Suit::Commerce, 100));
            let foreign = crate::NextEvidenceRecordId::<Test>::get() - 1; // belongs to 2, not to 1
            // no ids → nothing applied
            assert_eq!(Reputation::rectify_for_verdict(&1, Suit::Commerce, &[], 10_000, 2), 0);
            // someone ELSE's record → nothing applied, record intact
            assert_eq!(Reputation::rectify_for_verdict(&1, Suit::Commerce, &[foreign], 10_000, 2), 0);
            assert!(Reputation::owns_evidence(foreign, &2u64, Suit::Commerce));
            // duplicated ids must not inflate the cap: 2's record counted once
            let applied =
                Reputation::rectify_for_verdict(&2, Suit::Commerce, &[foreign, foreign], 10_000, 2);
            assert_eq!(applied, 100);
        });
    }

    #[test]
    fn evidence_record_worth_decays_with_the_epochs() {
        // Acta J2: what a verdict can rectify is what the falsehood is worth TODAY (ρ^Δ), and fully
        // decayed records are swept as dust by the bounded per-epoch sweep.
        new_test_ext().execute_with(|| {
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, Suit::Commerce, 100));
            let rec = crate::NextEvidenceRecordId::<Test>::get() - 1;
            Reputation::run_epoch(); // ρ = 0.5 in tests
            let (_, _, worth) = Reputation::evidence_record_current(rec).unwrap();
            assert_eq!(worth, 50, "record worth halves with the aggregate store");
            // run the evidence down to dust: the sweep prunes the dead record
            for _ in 0..10 {
                Reputation::run_epoch();
            }
            assert!(Reputation::evidence_record_current(rec).is_none(), "dust record swept");
        });
    }

    #[test]
    fn service_becomes_evidence_the_cold_start_pump() {
        // F2 piece 6 (the reviewer's spec): measured service turns into Technical evidence at the epoch
        // boundary — exactly once — and then decays like any evidence. THIS is how the first standing
        // enters a rep-zero chain (and how capa 5's own-evidence cap has something to stand on).
        new_test_ext().execute_with(|| {
            MockService::set(&vec![(0u64, 7u64, 5u64)]); // epoch 0: account 7 served 5 units
            System::set_block_number(10);
            Reputation::on_initialize(10u64); // boundary: epoch 0 ended → credited
            assert_eq!(Evidence::<Test>::get(7u64, Suit::Technical), 5);
            // and it entered THROUGH the journal (Acta J2: service evidence is disputable too)
            assert!(Reputation::evidence_record_current(0).is_some());
            // exactly-once + normal decay: the next boundary halves it and credits nothing new
            System::set_block_number(20);
            Reputation::on_initialize(20u64);
            assert_eq!(Evidence::<Test>::get(7u64, Suit::Technical), 2, "decayed, not re-credited");
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

    // ───────────────────────── §1.4b entrenchment-guard WIRING ─────────────────────────
    // reputation-core already unit-tests the guard MATH; these prove the guard is actually CONNECTED to the
    // live epoch pipeline: fed by consensus_reputation (the same values the committee is weighted on),
    // stepping the on-chain counter, and flipping the halt flag node/finality.rs reads.

    #[test]
    fn entrenchment_guard_does_not_halt_the_balanced_honest_cohort() {
        new_test_ext().execute_with(|| {
            // 5 founders with EQUAL evidence in every suit → each holds ~1/5 = 20% of consensus reputation,
            // well under the 1/3 threshold. This is the live set (pre-measured at 5×0.20). The guard must
            // NEVER fire on it, however long it runs — the no-false-halt condition of the wiring contract.
            for _epoch in 0..8u32 {
                for who in 1u64..=5 {
                    for suit in Suit::ALL {
                        assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), who, suit, 1_000_000));
                    }
                }
                Reputation::run_epoch();
                assert_eq!(EntrenchmentCounter::<Test>::get(), 0, "balanced cohort must never accumulate");
                assert!(!EntrenchmentHalted::<Test>::get());
                assert!(!Reputation::entrenchment_halted());
            }
        });
    }

    #[test]
    fn entrenchment_guard_halts_on_sustained_single_entity_capture() {
        new_test_ext().execute_with(|| {
            // Account 1 hoards reputation (huge evidence) while 2 and 3 stay marginal → its single-entity
            // share sits far above 1/3. Held for ENTRENCHMENT_REQUIRED_EPOCHS (7) consecutive epochs, finality
            // must HALT — but NOT before the 7th (sustained entrenchment, not a transient spike).
            for epoch in 1..=7u32 {
                for suit in Suit::ALL {
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, suit, 1_000_000));
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, suit, 1));
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 3u64, suit, 1));
                }
                Reputation::run_epoch();
                if epoch < ENTRENCHMENT_REQUIRED_EPOCHS {
                    assert!(!EntrenchmentHalted::<Test>::get(), "must not halt before the 7th over-threshold epoch");
                }
            }
            assert_eq!(EntrenchmentCounter::<Test>::get(), ENTRENCHMENT_REQUIRED_EPOCHS);
            assert!(EntrenchmentHalted::<Test>::get());
            assert!(Reputation::entrenchment_halted(), "finality must be frozen after sustained capture");
        });
    }

    #[test]
    fn entrenchment_guard_self_heals_when_the_share_dilutes() {
        new_test_ext().execute_with(|| {
            // Drive account 1 into an entrenchment halt…
            for _ in 1..=ENTRENCHMENT_REQUIRED_EPOCHS {
                for suit in Suit::ALL {
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, suit, 1_000_000));
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, suit, 1));
                }
                Reputation::run_epoch();
            }
            assert!(Reputation::entrenchment_halted());
            // …then the network grows: honest peers onboard until no single entity holds > 1/3. Account 1
            // stops topping up; one epoch under the diluted share RESETS the counter → finality resumes on
            // its own, with no authority pressing a button (Art VI + IX + XII: self-healing, no king).
            for who in 2u64..=6 {
                for suit in Suit::ALL {
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), who, suit, 1_000_000));
                }
            }
            Reputation::run_epoch();
            assert_eq!(EntrenchmentCounter::<Test>::get(), 0, "share diluted → counter resets");
            assert!(!Reputation::entrenchment_halted(), "finality self-resumes when entrenchment ends");
        });
    }

    #[test]
    fn recovery_band_records_the_halt_window_for_the_node_checkpoint() {
        // The self-heal seam (iron test): a halt→recover cycle leaves a `(halt_start,
        // cleared_at)` band so the node re-anchors the stuck heights to the healthy H_r. A height inside
        // the band resolves to H_r; a height outside resolves to None (committee stays empty / normal).
        new_test_ext().execute_with(|| {
            System::set_block_number(100); // so halt_start > 0 (a height before the band is testable)
            // arm the halt
            for _ in 1..=ENTRENCHMENT_REQUIRED_EPOCHS {
                for suit in Suit::ALL {
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, suit, 1_000_000));
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, suit, 1));
                }
                Reputation::run_epoch();
            }
            assert!(Reputation::entrenchment_halted());
            let bands = EntrenchmentBands::<Test>::get();
            assert_eq!(bands.len(), 1, "one open band while frozen");
            let (halt_start, cleared) = bands[0];
            assert!(cleared.is_none(), "band still open (frozen)");
            // while frozen there is no recovery anchor yet → node keeps the empty committee
            assert_eq!(Reputation::entrenchment_recovery_for(halt_start + 1), None);
            // dilute → recover
            for who in 2u64..=6 {
                for suit in Suit::ALL {
                    assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), who, suit, 1_000_000));
                }
            }
            System::set_block_number(System::block_number() + 1);
            Reputation::run_epoch();
            assert!(!Reputation::entrenchment_halted());
            let bands = EntrenchmentBands::<Test>::get();
            let (start, cleared) = bands[0];
            let h_r = cleared.expect("band closed with a recovery height");
            // a stuck height INSIDE the band re-anchors to H_r; one before the band does not.
            assert_eq!(Reputation::entrenchment_recovery_for(start + 1), Some(h_r));
            assert_eq!(Reputation::entrenchment_recovery_for(h_r), Some(h_r));
            assert_eq!(Reputation::entrenchment_recovery_for(start.saturating_sub(1)), None);
            // a height AFTER recovery is normal (not in any band) → no re-anchor needed
            assert_eq!(Reputation::entrenchment_recovery_for(h_r + 100), None);
        });
    }
}
