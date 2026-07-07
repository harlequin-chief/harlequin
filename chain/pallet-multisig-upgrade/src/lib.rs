//! Harlequin multisig-upgrade pallet — **the bridge governance the v1 lacked** (SPEC-RELAUNCH §3).
//!
//! The hole that killed the v1 chain: the king's key was removed and nothing was crowned in its
//! place — no way to fix a broken runtime. This pallet is the bridge: a founders' multisig whose ONLY
//! power is `set_code` (upgrades). No power over funds, reputation, justice or the manifesto.
//!
//! ## The shape of the key (all of SPEC-RELAUNCH §3, wired)
//! - **3-of-5 for upgrades**, each one public: a proposal announces the code hash + its invariant
//!   declaration, sits through an OBJECTION WINDOW, and only then can be applied. Any single signer
//!   can veto a pending proposal (the mechanical objection; nodes can also just refuse the binary —
//!   verify-by-recompute is the outer guard).
//! - **FLAG-1 (the reviewer)**: a proposal carries an [`InvariantDeclaration`] — manifesto hash, monetary
//!   caps, entrenchment-guard constants, and "preserves this validator" — that MUST equal the sealed
//!   values on-chain ([`SealedInvariants`]). A lie in the declaration is rejected mechanically at
//!   propose; a lie in the *blob* (declaration honest, code dishonest) is exactly what the objection
//!   window + every node's offline recompute of the blob is for. HONEST LIMIT, stated plainly: code
//!   replaces code — no runtime can mechanically prove properties of its successor. What this buys is
//!   that a treacherous upgrade must lie IN PUBLIC, on-chain, attributably, with a window to refuse it.
//! - **HARD expiry**: the key lives [`Config::InitialLife`] blocks (DECIDED: 12 months) and then stops
//!   applying upgrades. **Burn**: 3 approvals burn it forever, irreversibly, early (the safe direction
//!   needs no window).
//! - **Geometric renewal** (never orphaned, never perpetual): at/near expiry the key can be renewed for
//!   HALF the previous window — but it costs **4-of-5** AND a **co-signature of the majority of the
//!   current finality-grade set** ([`CommitteeInspect`]): an act of the NETWORK, not of the founders.
//!   The series ½+¼+… converges — the key mathematically cannot be perpetual (Art. VI in arithmetic).
//! - **Disaster renewal** (R2): when the chain is broken enough that no committee can be sampled for
//!   [`Config::DisasterEpochs`] consecutive epochs (objective on-chain metric, self-evaluated), renewal
//!   is possible at **5-of-5** (unanimity) for a QUARTER window, `set_code` scope only. Even the worst
//!   case repairs without crowning anyone (¼ < ½: still converging).
//! - **Seat cession**: a signer may cede its own seat to a member of high standing
//!   ([`StandingInspect`], floor = the network's own p25 bar) through the same public
//!   propose/approve/window process. Founders hold seats only because at block 0 nobody else existed.
//! - **What it can NEVER do**: touch the sealed manifesto, the monetary caps, the entrenchment guard
//!   or anyone's property — those are constants/state outside its one verb, and its one verb is public
//!   and refusable.
//!
//! Weights are placeholders (benchmark pre-mainnet), like the sibling pallets.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

/// FLAG-1: the invariant set a candidate upgrade must declare, checked against the sealed on-chain
/// values at propose time. Field-for-field the acta's list: manifesto, caps, guard, recursion.
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
    Debug,
)]
pub struct InvariantDeclaration {
    /// The sealed founding manifesto's hash (block 0, Art. XII). The candidate must carry it unchanged.
    pub manifesto_hash: [u8; 32],
    /// HLQ hard cap (540M × unit) — the candidate must keep it.
    pub cap_hlq: u128,
    /// SOV hard cap (54M × unit) — the candidate must keep it.
    pub cap_sov: u128,
    /// Entrenchment-guard halt threshold (≈FP_SCALE/3), sealed with the code (Art. XII).
    pub entrenchment_threshold_fp: i128,
    /// Entrenchment-guard run length (7 epochs), sealed with the code.
    pub entrenchment_required_epochs: u32,
    /// The recursive invariant: the candidate keeps THIS validator (declaring `false` is auto-refusal).
    pub preserves_validator: bool,
}

/// The sealed values the chain currently holds, composed by the runtime from their real sources
/// (manifesto pallet storage, token curve constants, reputation guard constants). `None` when the
/// truth does not exist yet (unsealed manifesto): proposing then FAILS CLOSED — a security source
/// must never degrade to a zero placeholder an attacker can simply declare (review).
pub trait SealedInvariants {
    fn current() -> Option<InvariantDeclaration>;
}

/// The current finality-grade co-signing set (SPEC §3: "co-firma de la mayoría del comité vigente").
/// The runtime wires the best on-chain-derivable set; an EMPTY vec means "no committee samplable this
/// epoch" and feeds the disaster-mode metric.
pub trait CommitteeInspect<AccountId> {
    fn committee() -> alloc::vec::Vec<AccountId>;
}

/// Standing source for seat cession ("miembros de alta reputación"): the runtime wires it to the
/// reputation pallet. `high_floor` is the network's own bar (the p25 attest floor — one bar, reused,
/// no new discretion).
pub trait StandingInspect<AccountId> {
    fn standing_of(who: &AccountId) -> i128;
    fn high_floor() -> i128;
}

/// The one verb: the runtime wires this to `frame_system::set_code` with Root origin. Seam-shaped so
/// tests exercise the full lifecycle without a real WASM blob.
pub trait SetCode {
    fn set_code(code: alloc::vec::Vec<u8>) -> frame::prelude::DispatchResult;
}

#[frame::pallet]
pub mod pallet {
    use super::{CommitteeInspect, InvariantDeclaration, SealedInvariants, SetCode, StandingInspect};
    use alloc::vec::Vec;
    use frame::prelude::*;

    /// Seats on the key. HARD constant (the "5" of 3-of-5/4-of-5/5-of-5): thresholds are constitutional
    /// arithmetic, not dials — a governance-mutable threshold is how a cabal lowers the bar (Art. XII).
    pub const SEATS: u32 = 5;
    /// Approvals to apply an upgrade (and to burn the key — the safe direction costs the same).
    pub const THRESHOLD_UPGRADE: u32 = 3;
    /// Approvals to renew the key (more than operating it: renewal is the suspicious act).
    pub const THRESHOLD_RENEWAL: u32 = 4;
    /// Approvals for a DISASTER renewal: unanimity, the highest possible bar.
    pub const THRESHOLD_DISASTER: u32 = SEATS;
    /// Bound of the committee co-sign list per renewal (a finality committee is ~tens).
    pub const MAX_COSIGNS: u32 = 256;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// The sealed invariant values (FLAG-1), composed by the runtime from their real sources.
        type Sealed: SealedInvariants;

        /// The current finality-grade co-signing set (empty = no committee samplable → disaster metric).
        type Committee: CommitteeInspect<Self::AccountId>;

        /// Standing source for seat cession.
        type Standing: StandingInspect<Self::AccountId>;

        /// The one verb (runtime: `frame_system::set_code` as Root — which itself checks spec_name and
        /// version bump on the candidate blob).
        type CodeSetter: SetCode;

        /// Initial life of the key in blocks (DECIDED, the maintainer: 12 months). Renewals halve it.
        #[pallet::constant]
        type InitialLife: Get<BlockNumberFor<Self>>;

        /// Public objection window between a proposal's announcement and the earliest apply.
        #[pallet::constant]
        type ObjectionWindow: Get<BlockNumberFor<Self>>;

        /// Blocks per epoch (must MATCH the reputation pallet's cadence — one clock, no drift).
        #[pallet::constant]
        type EpochLength: Get<BlockNumberFor<Self>>;

        /// Consecutive committee-less epochs that enable DISASTER renewal (PARÁMETRO, default 14).
        #[pallet::constant]
        type DisasterEpochs: Get<u32>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// The seats. Seeded at genesis with the 5 founders; changed only by public seat cession.
    #[pallet::storage]
    pub type Signers<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, ConstU32<SEATS>>, ValueQuery>;

    /// Block at which the key stops applying upgrades. Extended only by renewal (halving windows).
    #[pallet::storage]
    pub type LifeEnd<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    /// The CURRENT window length — the geometric term. Next normal renewal grants half of this,
    /// a disaster renewal a quarter. Genesis: [`Config::InitialLife`].
    #[pallet::storage]
    pub type CurrentWindow<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    /// The irreversible burn (SPEC §3: "se quema sola"). Once true, NOBODY has the key — forever.
    #[pallet::storage]
    pub type Burned<T> = StorageValue<_, bool, ValueQuery>;

    /// A pending upgrade proposal (one at a time: upgrades are serialized, public, unhurried).
    #[pallet::storage]
    pub type PendingUpgrade<T: Config> = StorageValue<
        _,
        (
            [u8; 32],                                    // code hash (blake2-256 of the raw blob)
            InvariantDeclaration,                        // FLAG-1 declaration (pre-validated)
            BlockNumberFor<T>,                           // announced_at
            BoundedVec<T::AccountId, ConstU32<SEATS>>,   // approvals
        ),
        OptionQuery,
    >;

    /// Approvals to burn the key early (no window: renouncing power is always safe).
    #[pallet::storage]
    pub type BurnApprovals<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, ConstU32<SEATS>>, ValueQuery>;

    /// A pending renewal: `(disaster, proposed_at, approvals, committee co-signs, committee SNAPSHOT)`.
    /// The ratifying set is SNAPSHOTTED when the renewal opens (the reviewer's four-eyes, anti-grinding): a
    /// fixed, deterministic electorate — reshaping reputation/beacon mid-vote can neither add friendly
    /// co-signers nor shrink the majority's denominator.
    #[pallet::storage]
    pub type PendingRenewal<T: Config> = StorageValue<
        _,
        (
            bool,
            BlockNumberFor<T>,
            BoundedVec<T::AccountId, ConstU32<SEATS>>,
            BoundedVec<T::AccountId, ConstU32<MAX_COSIGNS>>,
            BoundedVec<T::AccountId, ConstU32<MAX_COSIGNS>>,
        ),
        OptionQuery,
    >;

    /// (the reviewer's four-eyes, the hard question) Hashes whose proposal was VETOED once. A veto is per-
    /// proposal and SPENT: re-proposing the same hash escalates the apply threshold to 4-of-5, and no
    /// second veto exists — one coerced or lost seat can force the harder round, never block forever
    /// (the v1 death — an un-upgradeable chain — has no door here). Two dissenting seats DO block
    /// (4-of-5 unreachable): that is a real minority, not a hostage.
    #[pallet::storage]
    pub type VetoSpent<T: Config> = StorageMap<_, Blake2_128Concat, [u8; 32], (), OptionQuery>;

    /// A pending seat cession: `(from, to, announced_at, approvals)` — same public window discipline.
    #[pallet::storage]
    pub type PendingCession<T: Config> = StorageValue<
        _,
        (
            T::AccountId,
            T::AccountId,
            BlockNumberFor<T>,
            BoundedVec<T::AccountId, ConstU32<SEATS>>,
        ),
        OptionQuery,
    >;

    /// DISASTER metric (R2): consecutive epochs in which NO committee could be sampled (empty
    /// [`CommitteeInspect::committee`] at the epoch boundary). Self-evaluated by the chain's own
    /// clock — nobody declares an emergency; a sampled committee resets it to 0 on its own.
    #[pallet::storage]
    pub type EpochsWithoutCommittee<T> = StorageValue<_, u32, ValueQuery>;

    #[pallet::genesis_config]
    #[derive(frame::prelude::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        /// The 5 founding seats. Exactly [`SEATS`] accounts (genesis panics otherwise: the thresholds'
        /// arithmetic assumes 5 — booting with fewer would silently lower every bar).
        pub signers: alloc::vec::Vec<T::AccountId>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            if self.signers.is_empty() {
                // No signers configured (e.g. bare dev genesis): the pallet boots inert — no key was
                // ever minted, nothing can be proposed. LifeEnd 0 + empty seats = permanently expired.
                return;
            }
            assert!(
                self.signers.len() == SEATS as usize,
                "multisig genesis: exactly 5 seats (the thresholds' arithmetic is sealed to 5)"
            );
            let mut seen = alloc::collections::BTreeSet::new();
            for s in &self.signers {
                assert!(seen.insert(s), "multisig genesis: duplicate signer");
            }
            let bounded: BoundedVec<T::AccountId, ConstU32<SEATS>> =
                self.signers.clone().try_into().expect("len checked == SEATS; qed");
            Signers::<T>::put(bounded);
            LifeEnd::<T>::put(T::InitialLife::get());
            CurrentWindow::<T>::put(T::InitialLife::get());
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// An upgrade was proposed: hash + declaration public, objection window open (Art. X).
        UpgradeProposed { code_hash: [u8; 32], announced_at: BlockNumberFor<T> },
        /// A signer approved the pending upgrade.
        UpgradeApproved { code_hash: [u8; 32], approvals: u32 },
        /// A signer vetoed the pending upgrade (the mechanical objection).
        UpgradeVetoed { code_hash: [u8; 32], by: T::AccountId },
        /// The upgrade was applied (`set_code` dispatched; the version bump lives in the new blob).
        UpgradeApplied { code_hash: [u8; 32] },
        /// A signer approved burning the key.
        BurnApproved { approvals: u32 },
        /// The key is BURNED — irreversible; from this block nobody holds it, founders included.
        KeyBurned { at: BlockNumberFor<T> },
        /// A renewal was proposed (`disaster` = the R2 path).
        RenewalProposed { disaster: bool },
        /// A committee member co-signed the pending renewal (the network's ratification).
        RenewalCosigned { cosigns: u32 },
        /// The key was renewed: new expiry + the halved (or quartered) window. Ever smaller.
        KeyRenewed { life_end: BlockNumberFor<T>, window: BlockNumberFor<T>, disaster: bool },
        /// A seat cession was proposed (public window, like everything else).
        CessionProposed { from: T::AccountId, to: T::AccountId },
        /// A seat changed hands: the bridge opens to the network as the network grows people of standing.
        SeatCeded { from: T::AccountId, to: T::AccountId },
        /// The disaster metric ticked (empty committee at an epoch boundary) or reset.
        CommitteeSampleMetric { consecutive_missing: u32 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// The caller does not hold a seat.
        NotSigner,
        /// The key was burned — it does not exist anymore, for anyone, forever.
        Burnt,
        /// The key's life has expired (renewal is the only remaining verb).
        Expired,
        /// A proposal of this kind is already pending (serialized, public, one at a time).
        AlreadyPending,
        /// No such pending proposal.
        NothingPending,
        /// The pending proposal's hash does not match.
        HashMismatch,
        /// FLAG-1: the declared invariants do not equal the sealed on-chain values.
        InvariantsMismatch,
        /// The objection window has not elapsed yet.
        ObjectionWindowOpen,
        /// Not enough approvals for this action's threshold.
        NotEnoughApprovals,
        /// This signer already approved.
        AlreadyApproved,
        /// The provided code does not hash to the announced hash.
        CodeMismatch,
        /// Renewal is not due (the key is alive with more than a window to spare).
        RenewalNotDue,
        /// Disaster renewal needs the objective metric (N committee-less epochs) — it is not met.
        NoDisaster,
        /// The caller is not in the renewal's snapshotted co-signing set.
        NotCommittee,
        /// This hash's one veto was already spent — re-proposal escalates to 4-of-5 instead.
        VetoExhausted,
        /// The co-sign list is full (bounded storage; the snapshot electorate is capped upstream).
        TooManyCosigns,
        /// The chain holds no sealed invariants (unsealed manifesto) — upgrades are impossible until
        /// the truth to validate against exists. Fail CLOSED, never against a zero placeholder.
        InvariantsUnavailable,
        /// Not enough committee co-signatures (majority of the current set).
        NotEnoughCosigns,
        /// The cession target's standing is below the network's bar.
        LowStanding,
        /// The cession target already holds a seat.
        AlreadySigner,
        /// Only the seat's own holder may cede it.
        NotYourSeat,
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// Epoch-boundary tick of the DISASTER metric: sample the committee; empty → the counter
        /// climbs, sampled → it resets. Objective, self-evaluated, nobody's declaration (R2).
        fn on_initialize(n: BlockNumberFor<T>) -> Weight {
            let len = T::EpochLength::get();
            if !n.is_zero() && !len.is_zero() && (n % len).is_zero() {
                let missing = T::Committee::committee().is_empty();
                let prev = EpochsWithoutCommittee::<T>::get();
                let count = if missing { prev.saturating_add(1) } else { 0 };
                EpochsWithoutCommittee::<T>::put(count);
                // Emit only on signal (climbing) or on the recovery TRANSITION — a healthy chain's
                // steady state is silence, not an event every epoch (review).
                if missing || prev != 0 {
                    Self::deposit_event(Event::CommitteeSampleMetric {
                        consecutive_missing: count,
                    });
                }
            }
            Weight::from_parts(10_000_000, 0)
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Propose an upgrade: announce the candidate blob's blake2-256 hash + its FLAG-1 invariant
        /// declaration. The declaration is validated against the sealed values NOW — a mismatch never
        /// even opens a window. Public from this block (Art. X).
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(100_000, 0))]
        pub fn propose_upgrade(
            origin: OriginFor<T>,
            code_hash: [u8; 32],
            declared: InvariantDeclaration,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_signer(&who)?;
            Self::ensure_alive()?;
            ensure!(PendingUpgrade::<T>::get().is_none(), Error::<T>::AlreadyPending);
            // FLAG-1, the mechanical half: the declaration must equal the sealed truth, and the
            // candidate must declare it preserves this very validator (the recursive invariant).
            // No sealed truth on this chain → fail CLOSED.
            let sealed = T::Sealed::current().ok_or(Error::<T>::InvariantsUnavailable)?;
            ensure!(
                declared == sealed && declared.preserves_validator,
                Error::<T>::InvariantsMismatch
            );
            let now = frame_system::Pallet::<T>::block_number();
            let approvals: BoundedVec<T::AccountId, ConstU32<SEATS>> =
                alloc::vec![who].try_into().expect("1 <= SEATS; qed");
            PendingUpgrade::<T>::put((code_hash, declared, now, approvals));
            Self::deposit_event(Event::UpgradeProposed { code_hash, announced_at: now });
            Ok(())
        }

        /// Approve the pending upgrade (a seat's signature).
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn approve_upgrade(origin: OriginFor<T>, code_hash: [u8; 32]) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_signer(&who)?;
            Self::ensure_alive()?;
            PendingUpgrade::<T>::try_mutate(|maybe| {
                let (hash, _, _, approvals) =
                    maybe.as_mut().ok_or(Error::<T>::NothingPending)?;
                ensure!(*hash == code_hash, Error::<T>::HashMismatch);
                ensure!(!approvals.contains(&who), Error::<T>::AlreadyApproved);
                approvals.try_push(who).map_err(|_| Error::<T>::AlreadyApproved)?;
                Self::deposit_event(Event::UpgradeApproved {
                    code_hash,
                    approvals: approvals.len() as u32,
                });
                Ok(())
            })
        }

        /// VETO the pending upgrade — any single seat, no threshold, no window. Objection is cheap by
        /// design: stopping power must always cost less than wielding it. But a veto is SPENT once per
        /// hash: the proposal may be re-proposed, and the re-proposed round applies at 4-of-5 with no
        /// veto left — one seat can force the harder round, never hold the chain hostage forever.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn veto_upgrade(origin: OriginFor<T>, code_hash: [u8; 32]) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_signer(&who)?;
            let (hash, _, _, _) = PendingUpgrade::<T>::get().ok_or(Error::<T>::NothingPending)?;
            ensure!(hash == code_hash, Error::<T>::HashMismatch);
            ensure!(VetoSpent::<T>::get(code_hash).is_none(), Error::<T>::VetoExhausted);
            VetoSpent::<T>::insert(code_hash, ());
            PendingUpgrade::<T>::kill();
            Self::deposit_event(Event::UpgradeVetoed { code_hash, by: who });
            Ok(())
        }

        /// Apply the approved upgrade: anyone may execute once (i) 3-of-5 approved, (ii) the objection
        /// window elapsed, (iii) the blob hashes to the announced hash, (iv) the key is alive. The
        /// dispatch itself re-checks spec_name/version-bump on the blob (frame_system's own guards).
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(1_000_000, 0))]
        pub fn apply_upgrade(origin: OriginFor<T>, code: Vec<u8>) -> DispatchResult {
            ensure_signed(origin)?;
            Self::ensure_alive()?;
            let (hash, _, announced_at, approvals) =
                PendingUpgrade::<T>::get().ok_or(Error::<T>::NothingPending)?;
            // An once-vetoed hash applies at the ESCALATED threshold (4-of-5) — the veto's lasting
            // effect is a higher bar, not a locked door.
            let required = if VetoSpent::<T>::get(hash).is_some() {
                THRESHOLD_RENEWAL
            } else {
                THRESHOLD_UPGRADE
            };
            ensure!(Self::live_approvals(&approvals) >= required, Error::<T>::NotEnoughApprovals);
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(
                now >= announced_at.saturating_add(T::ObjectionWindow::get()),
                Error::<T>::ObjectionWindowOpen
            );
            ensure!(frame::hashing::blake2_256(&code) == hash, Error::<T>::CodeMismatch);
            T::CodeSetter::set_code(code)?;
            PendingUpgrade::<T>::kill();
            Self::deposit_event(Event::UpgradeApplied { code_hash: hash });
            Ok(())
        }

        /// Approve burning the key. At 3 approvals it burns — IRREVERSIBLY, immediately, no window:
        /// renouncing power is the one act that needs no protection from haste.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn approve_burn(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_signer(&who)?;
            ensure!(!Burned::<T>::get(), Error::<T>::Burnt);
            let count = BurnApprovals::<T>::try_mutate(|approvals| -> Result<u32, DispatchError> {
                ensure!(!approvals.contains(&who), Error::<T>::AlreadyApproved);
                approvals.try_push(who).map_err(|_| Error::<T>::AlreadyApproved)?;
                // Burning is irreversible: only approvals of CURRENT seats count toward the threshold.
                Ok(Self::live_approvals(approvals))
            })?;
            Self::deposit_event(Event::BurnApproved { approvals: count });
            if count >= THRESHOLD_UPGRADE {
                Burned::<T>::put(true);
                PendingUpgrade::<T>::kill();
                PendingRenewal::<T>::kill();
                PendingCession::<T>::kill();
                Self::deposit_event(Event::KeyBurned {
                    at: frame_system::Pallet::<T>::block_number(),
                });
            }
            Ok(())
        }

        /// Propose renewing the key (SPEC §3). Normal path: only when the key is within one objection
        /// window of expiry, or already expired — renewal is a deathbed act, not a routine one.
        /// Disaster path (R2): only when the objective metric holds (no committee for N epochs).
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(100_000, 0))]
        pub fn propose_renewal(origin: OriginFor<T>, disaster: bool) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_signer(&who)?;
            ensure!(!Burned::<T>::get(), Error::<T>::Burnt);
            // A STALE pending renewal (older than the TTL) is displaced, never a hostage.
            if let Some((_, proposed_at, _, _, _)) = PendingRenewal::<T>::get() {
                ensure!(Self::is_stale(proposed_at), Error::<T>::AlreadyPending);
                PendingRenewal::<T>::kill();
            }
            let now = frame_system::Pallet::<T>::block_number();
            if disaster {
                ensure!(
                    EpochsWithoutCommittee::<T>::get() >= T::DisasterEpochs::get(),
                    Error::<T>::NoDisaster
                );
            } else {
                let due_from = LifeEnd::<T>::get().saturating_sub(T::ObjectionWindow::get());
                ensure!(now >= due_from, Error::<T>::RenewalNotDue);
            }
            let approvals: BoundedVec<T::AccountId, ConstU32<SEATS>> =
                alloc::vec![who].try_into().expect("1 <= SEATS; qed");
            // SNAPSHOT the ratifying set at open (anti-grinding): a fixed electorate for this renewal.
            // For a NORMAL renewal an empty snapshot means no committee exists → unratifiable now
            // (that is what the disaster path is for), so refuse early rather than mint a dead proposal.
            let snapshot: BoundedVec<T::AccountId, ConstU32<MAX_COSIGNS>> = {
                let mut set = T::Committee::committee();
                set.truncate(MAX_COSIGNS as usize);
                set.try_into().expect("truncated to bound; qed")
            };
            if !disaster {
                ensure!(!snapshot.is_empty(), Error::<T>::NotEnoughCosigns);
            }
            PendingRenewal::<T>::put((disaster, now, approvals, BoundedVec::default(), snapshot));
            Self::deposit_event(Event::RenewalProposed { disaster });
            Ok(())
        }

        /// Approve the pending renewal (a seat's signature).
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn approve_renewal(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_signer(&who)?;
            ensure!(!Burned::<T>::get(), Error::<T>::Burnt);
            PendingRenewal::<T>::try_mutate(|maybe| {
                let (_, _, approvals, _, _) = maybe.as_mut().ok_or(Error::<T>::NothingPending)?;
                ensure!(!approvals.contains(&who), Error::<T>::AlreadyApproved);
                approvals.try_push(who).map_err(|_| Error::<T>::AlreadyApproved)?;
                Ok(())
            })
        }

        /// Co-sign the pending NORMAL renewal as a member of its SNAPSHOTTED ratifying set — the
        /// network's ratification (the set is drawn by reputation sortition; the founders do not
        /// choose it, and it was frozen when the renewal opened so it cannot be reshaped mid-vote).
        /// Disaster renewals take no co-signs: their premise is that no committee exists.
        #[pallet::call_index(7)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn cosign_renewal(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(!Burned::<T>::get(), Error::<T>::Burnt);
            PendingRenewal::<T>::try_mutate(|maybe| {
                let (disaster, _, _, cosigns, snapshot) =
                    maybe.as_mut().ok_or(Error::<T>::NothingPending)?;
                ensure!(!*disaster, Error::<T>::NoDisaster);
                ensure!(snapshot.contains(&who), Error::<T>::NotCommittee);
                ensure!(!cosigns.contains(&who), Error::<T>::AlreadyApproved);
                cosigns.try_push(who).map_err(|_| Error::<T>::TooManyCosigns)?;
                Self::deposit_event(Event::RenewalCosigned { cosigns: cosigns.len() as u32 });
                Ok(())
            })
        }

        /// Execute the renewal. Normal: 4-of-5 + a strict majority of the CURRENT committee's
        /// co-signatures → the window HALVES. Disaster: 5-of-5 + the metric still holding → the window
        /// QUARTERS. Either way the new life runs from `max(now, old end)` and the series converges:
        /// arithmetic, not promises, keeps the key from being perpetual.
        #[pallet::call_index(8)]
        #[pallet::weight(Weight::from_parts(200_000, 0))]
        pub fn apply_renewal(origin: OriginFor<T>) -> DispatchResult {
            ensure_signed(origin)?;
            ensure!(!Burned::<T>::get(), Error::<T>::Burnt);
            let (disaster, proposed_at, approvals, cosigns, snapshot) =
                PendingRenewal::<T>::get().ok_or(Error::<T>::NothingPending)?;
            if disaster {
                ensure!(
                    Self::live_approvals(&approvals) >= THRESHOLD_DISASTER,
                    Error::<T>::NotEnoughApprovals
                );
                ensure!(
                    EpochsWithoutCommittee::<T>::get() >= T::DisasterEpochs::get(),
                    Error::<T>::NoDisaster
                );
                // No objection window here — the premise is a broken chain and there is no committee
                // left to observe one; unanimity + the objective metric are the whole guard.
            } else {
                ensure!(
                    Self::live_approvals(&approvals) >= THRESHOLD_RENEWAL,
                    Error::<T>::NotEnoughApprovals
                );
                // Same public-window discipline as upgrades and cessions (review): a
                // renewal is observable before it lands. Proposable within a window of expiry, so at
                // worst it executes just past the old end — renewal-after-expiry is legal by design.
                ensure!(
                    frame_system::Pallet::<T>::block_number()
                        >= proposed_at.saturating_add(T::ObjectionWindow::get()),
                    Error::<T>::ObjectionWindowOpen
                );
                // Strict majority of the SNAPSHOTTED ratifying set (fixed when the renewal opened —
                // deterministic electorate, no mid-vote reshaping). Every cosign was already checked
                // against the snapshot at signing.
                ensure!(
                    !snapshot.is_empty()
                        && (cosigns.len() as u32) * 2 > snapshot.len() as u32,
                    Error::<T>::NotEnoughCosigns
                );
            }
            let old_window = CurrentWindow::<T>::get();
            let divisor: u32 = if disaster { 4 } else { 2 };
            let window = old_window / divisor.into();
            let window = window.max(One::one()); // a zero window would be a silent burn — floor at 1
            let now = frame_system::Pallet::<T>::block_number();
            let life_end = now.max(LifeEnd::<T>::get()).saturating_add(window);
            LifeEnd::<T>::put(life_end);
            CurrentWindow::<T>::put(window);
            PendingRenewal::<T>::kill();
            Self::deposit_event(Event::KeyRenewed { life_end, window, disaster });
            Ok(())
        }

        /// Propose ceding YOUR seat to a member of high standing (the network's own p25 bar). Same
        /// public discipline: 3-of-5 + objection window. The founders' names are an accident of block
        /// 0, not a rank — the bridge opens to the network as the network grows people of standing.
        #[pallet::call_index(9)]
        #[pallet::weight(Weight::from_parts(100_000, 0))]
        pub fn propose_cession(origin: OriginFor<T>, to: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_signer(&who)?;
            ensure!(!Burned::<T>::get(), Error::<T>::Burnt);
            // A STALE pending cession (older than the TTL) is displaced, never a hostage.
            if let Some((_, _, announced_at, _)) = PendingCession::<T>::get() {
                ensure!(Self::is_stale(announced_at), Error::<T>::AlreadyPending);
                PendingCession::<T>::kill();
            }
            ensure!(!Signers::<T>::get().contains(&to), Error::<T>::AlreadySigner);
            ensure!(
                T::Standing::standing_of(&to) >= T::Standing::high_floor()
                    && T::Standing::standing_of(&to) > 0,
                Error::<T>::LowStanding
            );
            let now = frame_system::Pallet::<T>::block_number();
            let approvals: BoundedVec<T::AccountId, ConstU32<SEATS>> =
                alloc::vec![who.clone()].try_into().expect("1 <= SEATS; qed");
            PendingCession::<T>::put((who.clone(), to.clone(), now, approvals));
            Self::deposit_event(Event::CessionProposed { from: who, to });
            Ok(())
        }

        /// Approve the pending cession (a seat's signature).
        #[pallet::call_index(10)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn approve_cession(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::ensure_signer(&who)?;
            ensure!(!Burned::<T>::get(), Error::<T>::Burnt);
            PendingCession::<T>::try_mutate(|maybe| {
                let (_, _, _, approvals) = maybe.as_mut().ok_or(Error::<T>::NothingPending)?;
                ensure!(!approvals.contains(&who), Error::<T>::AlreadyApproved);
                approvals.try_push(who).map_err(|_| Error::<T>::AlreadyApproved)?;
                Ok(())
            })
        }

        /// Execute the cession after 3-of-5 + the window: the seat changes hands. The ceding signer's
        /// approval still counts (it proposed); the standing bar is re-checked at execution.
        #[pallet::call_index(11)]
        #[pallet::weight(Weight::from_parts(100_000, 0))]
        pub fn apply_cession(origin: OriginFor<T>) -> DispatchResult {
            ensure_signed(origin)?;
            ensure!(!Burned::<T>::get(), Error::<T>::Burnt);
            let (from, to, announced_at, approvals) =
                PendingCession::<T>::get().ok_or(Error::<T>::NothingPending)?;
            ensure!(
                approvals.len() as u32 >= THRESHOLD_UPGRADE,
                Error::<T>::NotEnoughApprovals
            );
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(
                now >= announced_at.saturating_add(T::ObjectionWindow::get()),
                Error::<T>::ObjectionWindowOpen
            );
            ensure!(
                T::Standing::standing_of(&to) >= T::Standing::high_floor()
                    && T::Standing::standing_of(&to) > 0,
                Error::<T>::LowStanding
            );
            ensure!(!Signers::<T>::get().contains(&to), Error::<T>::AlreadySigner);
            Signers::<T>::mutate(|signers| {
                if let Some(slot) = signers.iter_mut().find(|s| **s == from) {
                    *slot = to.clone();
                }
            });
            PendingCession::<T>::kill();
            Self::deposit_event(Event::SeatCeded { from, to });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        fn ensure_signer(who: &T::AccountId) -> DispatchResult {
            ensure!(Signers::<T>::get().contains(who), Error::<T>::NotSigner);
            Ok(())
        }

        /// Approvals that STILL belong to a current seat (review: a signer that approved and
        /// then ceded its seat must not keep counting — thresholds are of the seats as they are NOW,
        /// or a rotating seat could stack ghost approvals below the real bar).
        fn live_approvals(approvals: &[T::AccountId]) -> u32 {
            let signers = Signers::<T>::get();
            approvals.iter().filter(|a| signers.contains(a)).count() as u32
        }

        /// A pending proposal older than this is STALE and may be displaced by a new one — the
        /// single-slot pending guard can never hold the key hostage (review: a renewal or
        /// cession proposed with the wrong shape and never completed would otherwise block the slot
        /// forever; near expiry that bricks the key — the v1 death by a third door).
        fn pending_ttl() -> BlockNumberFor<T> {
            T::ObjectionWindow::get().saturating_mul(4u32.into())
        }

        fn is_stale(proposed_at: BlockNumberFor<T>) -> bool {
            frame_system::Pallet::<T>::block_number()
                > proposed_at.saturating_add(Self::pending_ttl())
        }

        /// Alive = not burned and not expired. Only upgrades demand it; renewal has its own gates.
        fn ensure_alive() -> DispatchResult {
            ensure!(!Burned::<T>::get(), Error::<T>::Burnt);
            ensure!(
                frame_system::Pallet::<T>::block_number() < LifeEnd::<T>::get(),
                Error::<T>::Expired
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_multisig_upgrade;
    use crate::{
        Burned, CommitteeInspect, CurrentWindow, EpochsWithoutCommittee, Error, InvariantDeclaration,
        LifeEnd, SealedInvariants, SetCode, Signers, StandingInspect,
    };
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Multisig: pallet_multisig_upgrade,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    fn sealed() -> InvariantDeclaration {
        InvariantDeclaration {
            manifesto_hash: [7u8; 32],
            cap_hlq: 540,
            cap_sov: 54,
            entrenchment_threshold_fp: 333,
            entrenchment_required_epochs: 7,
            preserves_validator: true,
        }
    }

    pub struct MockSealed;
    impl SealedInvariants for MockSealed {
        fn current() -> Option<InvariantDeclaration> {
            Some(sealed())
        }
    }

    // Mutable mock committee (storage-backed parameter) so tests can empty it (disaster metric).
    parameter_types! {
        pub storage MockCommittee: Vec<u64> = vec![100, 101, 102];
        pub storage SetCodeLog: Vec<Vec<u8>> = Vec::new();
        pub storage MockStanding: Vec<(u64, i128)> = Vec::new();
    }
    pub struct Committee;
    impl CommitteeInspect<u64> for Committee {
        fn committee() -> Vec<u64> {
            MockCommittee::get()
        }
    }
    pub struct Standing;
    impl StandingInspect<u64> for Standing {
        fn standing_of(who: &u64) -> i128 {
            MockStanding::get().iter().find(|(a, _)| a == who).map(|(_, s)| *s).unwrap_or(0)
        }
        fn high_floor() -> i128 {
            50
        }
    }
    pub struct Setter;
    impl SetCode for Setter {
        fn set_code(code: Vec<u8>) -> DispatchResult {
            let mut v = SetCodeLog::get();
            v.push(code);
            SetCodeLog::set(&v);
            Ok(())
        }
    }

    impl pallet_multisig_upgrade::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type Sealed = MockSealed;
        type Committee = Committee;
        type Standing = Standing;
        type CodeSetter = Setter;
        type InitialLife = ConstU64<1000>;
        type ObjectionWindow = ConstU64<10>;
        type EpochLength = ConstU64<10>;
        type DisasterEpochs = ConstU32<3>;
    }

    const F: [u64; 5] = [1, 2, 3, 4, 5]; // the five founding seats

    fn new_test_ext() -> TestState {
        let mut ext: TestState = RuntimeGenesisConfig {
            system: Default::default(),
            multisig: pallet_multisig_upgrade::GenesisConfig { signers: F.to_vec() },
        }
        .build_storage()
        .unwrap()
        .into();
        ext.execute_with(|| System::set_block_number(1));
        ext
    }

    fn propose_ok(code: &[u8]) -> [u8; 32] {
        let hash = frame::hashing::blake2_256(code);
        assert_ok!(Multisig::propose_upgrade(RuntimeOrigin::signed(1), hash, sealed()));
        hash
    }

    #[test]
    fn upgrade_lifecycle_3_of_5_public_window() {
        new_test_ext().execute_with(|| {
            let code = b"new-runtime".to_vec();
            let hash = propose_ok(&code);
            // a second approval is not enough, and the window blocks even a third
            assert_ok!(Multisig::approve_upgrade(RuntimeOrigin::signed(2), hash));
            assert_noop!(
                Multisig::apply_upgrade(RuntimeOrigin::signed(9), code.clone()),
                Error::<Test>::NotEnoughApprovals
            );
            assert_ok!(Multisig::approve_upgrade(RuntimeOrigin::signed(3), hash));
            assert_noop!(
                Multisig::apply_upgrade(RuntimeOrigin::signed(9), code.clone()),
                Error::<Test>::ObjectionWindowOpen
            );
            // window elapses → anyone applies; the blob must match the announced hash
            System::set_block_number(12);
            assert_noop!(
                Multisig::apply_upgrade(RuntimeOrigin::signed(9), b"tampered".to_vec()),
                Error::<Test>::CodeMismatch
            );
            assert_ok!(Multisig::apply_upgrade(RuntimeOrigin::signed(9), code.clone()));
            assert_eq!(SetCodeLog::get(), vec![code]);
        });
    }

    #[test]
    fn flag1_invariant_lie_is_rejected_at_propose() {
        new_test_ext().execute_with(|| {
            let hash = [1u8; 32];
            // wrong cap
            let mut lie = sealed();
            lie.cap_hlq = 541;
            assert_noop!(
                Multisig::propose_upgrade(RuntimeOrigin::signed(1), hash, lie),
                Error::<Test>::InvariantsMismatch
            );
            // honest values but does not preserve the validator (the recursive invariant)
            let mut drop_validator = sealed();
            drop_validator.preserves_validator = false;
            assert_noop!(
                Multisig::propose_upgrade(RuntimeOrigin::signed(1), hash, drop_validator),
                Error::<Test>::InvariantsMismatch
            );
        });
    }

    #[test]
    fn any_single_signer_vetoes() {
        new_test_ext().execute_with(|| {
            let code = b"contested".to_vec();
            let hash = propose_ok(&code);
            assert_ok!(Multisig::approve_upgrade(RuntimeOrigin::signed(2), hash));
            assert_ok!(Multisig::approve_upgrade(RuntimeOrigin::signed(3), hash));
            // seat 5 alone kills it — objection is cheaper than power
            assert_ok!(Multisig::veto_upgrade(RuntimeOrigin::signed(5), hash));
            System::set_block_number(12);
            assert_noop!(
                Multisig::apply_upgrade(RuntimeOrigin::signed(9), code),
                Error::<Test>::NothingPending
            );
        });
    }

    #[test]
    fn veto_escalates_to_4_of_5_never_blocks_forever() {
        // the reviewer's hard question: one coerced/lost seat must not be able to hold the chain
        // un-upgradeable (the v1 death by another door). A veto is spent per hash: the re-proposed
        // round has no veto left and applies at the ESCALATED 4-of-5.
        new_test_ext().execute_with(|| {
            let code = b"contested-but-needed".to_vec();
            let hash = propose_ok(&code);
            assert_ok!(Multisig::veto_upgrade(RuntimeOrigin::signed(5), hash));
            // re-propose the SAME hash: the hostile seat's second veto does not exist
            let _ = propose_ok(&code);
            assert_noop!(
                Multisig::veto_upgrade(RuntimeOrigin::signed(5), hash),
                Error::<Test>::VetoExhausted
            );
            // 3 approvals are no longer enough (escalated round)…
            assert_ok!(Multisig::approve_upgrade(RuntimeOrigin::signed(2), hash));
            assert_ok!(Multisig::approve_upgrade(RuntimeOrigin::signed(3), hash));
            System::set_block_number(12);
            assert_noop!(
                Multisig::apply_upgrade(RuntimeOrigin::signed(9), code.clone()),
                Error::<Test>::NotEnoughApprovals
            );
            // …4-of-5 is: the escalation resolves, the chain stays upgradeable
            assert_ok!(Multisig::approve_upgrade(RuntimeOrigin::signed(4), hash));
            assert_ok!(Multisig::apply_upgrade(RuntimeOrigin::signed(9), code.clone()));
            assert_eq!(SetCodeLog::get(), vec![code]);
        });
    }

    #[test]
    fn renewal_electorate_is_snapshotted_at_open() {
        // Anti-grinding (the reviewer's four-eyes): the ratifying set is frozen when the renewal opens —
        // members added to the committee mid-vote cannot co-sign, and shrinking the committee
        // afterwards does not shrink the majority's denominator.
        new_test_ext().execute_with(|| {
            System::set_block_number(995);
            assert_ok!(Multisig::propose_renewal(RuntimeOrigin::signed(1), false));
            // 103 joins the live committee AFTER the snapshot → cannot co-sign this renewal
            MockCommittee::set(&vec![100, 101, 102, 103]);
            assert_noop!(
                Multisig::cosign_renewal(RuntimeOrigin::signed(103)),
                Error::<Test>::NotCommittee
            );
            // and the denominator stays the snapshot's 3: two of three co-signs still carry it
            for s in [2u64, 3, 4] {
                assert_ok!(Multisig::approve_renewal(RuntimeOrigin::signed(s)));
            }
            assert_ok!(Multisig::cosign_renewal(RuntimeOrigin::signed(100)));
            assert_ok!(Multisig::cosign_renewal(RuntimeOrigin::signed(101)));
            System::set_block_number(1005); // the renewal's own objection window elapses
            assert_ok!(Multisig::apply_renewal(RuntimeOrigin::signed(9)));
            assert_eq!(CurrentWindow::<Test>::get(), 500);
        });
    }

    #[test]
    fn only_signers_hold_the_pen() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                Multisig::propose_upgrade(RuntimeOrigin::signed(9), [1u8; 32], sealed()),
                Error::<Test>::NotSigner
            );
            let hash = propose_ok(b"x");
            assert_noop!(
                Multisig::approve_upgrade(RuntimeOrigin::signed(9), hash),
                Error::<Test>::NotSigner
            );
            assert_noop!(
                Multisig::veto_upgrade(RuntimeOrigin::signed(9), hash),
                Error::<Test>::NotSigner
            );
        });
    }

    #[test]
    fn expiry_kills_upgrades_but_not_renewal() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1000); // = LifeEnd → expired
            assert_noop!(
                Multisig::propose_upgrade(RuntimeOrigin::signed(1), [1u8; 32], sealed()),
                Error::<Test>::Expired
            );
            // renewal is exactly the verb that survives expiry
            assert_ok!(Multisig::propose_renewal(RuntimeOrigin::signed(1), false));
        });
    }

    #[test]
    fn burn_is_3_of_5_and_forever() {
        new_test_ext().execute_with(|| {
            assert_ok!(Multisig::approve_burn(RuntimeOrigin::signed(1)));
            assert_ok!(Multisig::approve_burn(RuntimeOrigin::signed(2)));
            assert!(!Burned::<Test>::get());
            assert_ok!(Multisig::approve_burn(RuntimeOrigin::signed(3)));
            assert!(Burned::<Test>::get());
            // nothing works anymore — not proposing, not renewing, not ceding. Nobody has the key.
            assert_noop!(
                Multisig::propose_upgrade(RuntimeOrigin::signed(1), [1u8; 32], sealed()),
                Error::<Test>::Burnt
            );
            assert_noop!(
                Multisig::propose_renewal(RuntimeOrigin::signed(1), false),
                Error::<Test>::Burnt
            );
            assert_noop!(
                Multisig::propose_cession(RuntimeOrigin::signed(1), 100),
                Error::<Test>::Burnt
            );
        });
    }

    #[test]
    fn renewal_needs_4_of_5_plus_committee_majority_and_halves() {
        new_test_ext().execute_with(|| {
            // not due while the key has more than a window of life left
            assert_noop!(
                Multisig::propose_renewal(RuntimeOrigin::signed(1), false),
                Error::<Test>::RenewalNotDue
            );
            System::set_block_number(995); // within ObjectionWindow(10) of LifeEnd(1000)
            assert_ok!(Multisig::propose_renewal(RuntimeOrigin::signed(1), false));
            assert_ok!(Multisig::approve_renewal(RuntimeOrigin::signed(2)));
            assert_ok!(Multisig::approve_renewal(RuntimeOrigin::signed(3)));
            // 3 approvals < 4 → refused even with cosigns
            assert_ok!(Multisig::cosign_renewal(RuntimeOrigin::signed(100)));
            assert_ok!(Multisig::cosign_renewal(RuntimeOrigin::signed(101)));
            assert_noop!(
                Multisig::apply_renewal(RuntimeOrigin::signed(9)),
                Error::<Test>::NotEnoughApprovals
            );
            assert_ok!(Multisig::approve_renewal(RuntimeOrigin::signed(4)));
            // the renewal is public too: its own objection window must elapse before it lands
            assert_noop!(
                Multisig::apply_renewal(RuntimeOrigin::signed(9)),
                Error::<Test>::ObjectionWindowOpen
            );
            System::set_block_number(1005); // proposed at 995 + window 10
            // 2 of 3 committee co-signs = strict majority → renews; window HALVES (1000 → 500)
            assert_ok!(Multisig::apply_renewal(RuntimeOrigin::signed(9)));
            assert_eq!(CurrentWindow::<Test>::get(), 500);
            assert_eq!(LifeEnd::<Test>::get(), 1005 + 500); // key had expired at 1000 → extends from now
        });
    }

    #[test]
    fn stale_approvals_die_with_the_ceded_seat() {
        // Review (core finding): an approval from a seat that later changed hands must not
        // keep counting — thresholds are of the seats as they are NOW.
        new_test_ext().execute_with(|| {
            let code = b"upgrade-x".to_vec();
            let hash = propose_ok(&code); // proposed (=approved) by seat 1
            assert_ok!(Multisig::approve_upgrade(RuntimeOrigin::signed(2), hash));
            assert_ok!(Multisig::approve_upgrade(RuntimeOrigin::signed(5), hash)); // 3 approvals
            // seat 5 cedes to 201 (reputable) through the public process
            MockStanding::set(&vec![(201u64, 80i128)]);
            assert_ok!(Multisig::propose_cession(RuntimeOrigin::signed(5), 201));
            assert_ok!(Multisig::approve_cession(RuntimeOrigin::signed(1)));
            assert_ok!(Multisig::approve_cession(RuntimeOrigin::signed(2)));
            System::set_block_number(12);
            assert_ok!(Multisig::apply_cession(RuntimeOrigin::signed(9)));
            // 5's old approval is now a ghost: only 2 live approvals remain → apply refused
            assert_noop!(
                Multisig::apply_upgrade(RuntimeOrigin::signed(9), code.clone()),
                Error::<Test>::NotEnoughApprovals
            );
            // the NEW seat approves on its own judgement → threshold is honestly met again
            assert_ok!(Multisig::approve_upgrade(RuntimeOrigin::signed(201), hash));
            assert_ok!(Multisig::apply_upgrade(RuntimeOrigin::signed(9), code));
        });
    }

    #[test]
    fn stale_pending_renewal_is_displaced_not_a_hostage() {
        // Review: a renewal proposed with the wrong shape and never completed must not
        // occupy the pending slot forever (near expiry that would brick the key).
        new_test_ext().execute_with(|| {
            System::set_block_number(995);
            assert_ok!(Multisig::propose_renewal(RuntimeOrigin::signed(1), false));
            // a fresh pending blocks a second proposal…
            assert_noop!(
                Multisig::propose_renewal(RuntimeOrigin::signed(2), false),
                Error::<Test>::AlreadyPending
            );
            // …but once stale (TTL = 4 × ObjectionWindow = 40 blocks), a new one displaces it
            System::set_block_number(995 + 41);
            assert_ok!(Multisig::propose_renewal(RuntimeOrigin::signed(2), false));
        });
    }

    #[test]
    fn renewal_without_committee_majority_fails() {
        new_test_ext().execute_with(|| {
            System::set_block_number(995);
            assert_ok!(Multisig::propose_renewal(RuntimeOrigin::signed(1), false));
            for s in [2u64, 3, 4] {
                assert_ok!(Multisig::approve_renewal(RuntimeOrigin::signed(s)));
            }
            // only 1 of 3 committee members co-signs → not a majority; and a non-member cannot co-sign
            assert_ok!(Multisig::cosign_renewal(RuntimeOrigin::signed(100)));
            assert_noop!(
                Multisig::cosign_renewal(RuntimeOrigin::signed(9)),
                Error::<Test>::NotCommittee
            );
            System::set_block_number(1005); // past the renewal's window: only the cosigns are missing
            assert_noop!(
                Multisig::apply_renewal(RuntimeOrigin::signed(9)),
                Error::<Test>::NotEnoughCosigns
            );
        });
    }

    #[test]
    fn disaster_renewal_needs_metric_unanimity_and_quarters() {
        new_test_ext().execute_with(|| {
            // metric not met → the disaster verb does not exist
            assert_noop!(
                Multisig::propose_renewal(RuntimeOrigin::signed(1), true),
                Error::<Test>::NoDisaster
            );
            // empty the committee and tick 3 epoch boundaries (EpochLength=10, DisasterEpochs=3)
            MockCommittee::set(&Vec::new());
            for boundary in [10u64, 20, 30] {
                System::set_block_number(boundary);
                Multisig::on_initialize(boundary);
            }
            assert_eq!(EpochsWithoutCommittee::<Test>::get(), 3);
            assert_ok!(Multisig::propose_renewal(RuntimeOrigin::signed(1), true));
            for s in [2u64, 3, 4] {
                assert_ok!(Multisig::approve_renewal(RuntimeOrigin::signed(s)));
            }
            // 4 of 5 is NOT enough in disaster — unanimity or nothing
            assert_noop!(
                Multisig::apply_renewal(RuntimeOrigin::signed(9)),
                Error::<Test>::NotEnoughApprovals
            );
            assert_ok!(Multisig::approve_renewal(RuntimeOrigin::signed(5)));
            assert_ok!(Multisig::apply_renewal(RuntimeOrigin::signed(9)));
            // window QUARTERS (1000 → 250); life extends from now (the key was not yet expired: from end)
            assert_eq!(CurrentWindow::<Test>::get(), 250);
            assert_eq!(LifeEnd::<Test>::get(), 1000 + 250);
        });
    }

    #[test]
    fn committee_recovery_resets_the_disaster_metric() {
        new_test_ext().execute_with(|| {
            MockCommittee::set(&Vec::new());
            for boundary in [10u64, 20] {
                System::set_block_number(boundary);
                Multisig::on_initialize(boundary);
            }
            assert_eq!(EpochsWithoutCommittee::<Test>::get(), 2);
            // a committee is sampled again → the metric resets BY ITSELF (nobody declares recovery)
            MockCommittee::set(&vec![100]);
            System::set_block_number(30);
            Multisig::on_initialize(30);
            assert_eq!(EpochsWithoutCommittee::<Test>::get(), 0);
            assert_noop!(
                Multisig::propose_renewal(RuntimeOrigin::signed(1), true),
                Error::<Test>::NoDisaster
            );
        });
    }

    #[test]
    fn seat_cession_needs_standing_3_of_5_and_window() {
        new_test_ext().execute_with(|| {
            // target below the network's bar → refused outright
            MockStanding::set(&vec![(200u64, 10i128), (201u64, 80i128)]);
            assert_noop!(
                Multisig::propose_cession(RuntimeOrigin::signed(5), 200),
                Error::<Test>::LowStanding
            );
            // reputable target: propose (seat 5 cedes its own), approve to 3, wait the window, apply
            assert_ok!(Multisig::propose_cession(RuntimeOrigin::signed(5), 201));
            assert_ok!(Multisig::approve_cession(RuntimeOrigin::signed(1)));
            assert_ok!(Multisig::approve_cession(RuntimeOrigin::signed(2)));
            assert_noop!(
                Multisig::apply_cession(RuntimeOrigin::signed(9)),
                Error::<Test>::ObjectionWindowOpen
            );
            System::set_block_number(12);
            assert_ok!(Multisig::apply_cession(RuntimeOrigin::signed(9)));
            let signers = Signers::<Test>::get();
            assert!(signers.contains(&201) && !signers.contains(&5), "seat changed hands");
            // the new seat signs like any founder ever did
            assert_ok!(Multisig::propose_upgrade(RuntimeOrigin::signed(201), [9u8; 32], sealed()));
        });
    }

    #[test]
    fn genesis_without_signers_boots_inert() {
        let mut ext: TestState = RuntimeGenesisConfig {
            system: Default::default(),
            multisig: pallet_multisig_upgrade::GenesisConfig { signers: vec![] },
        }
        .build_storage()
        .unwrap()
        .into();
        ext.execute_with(|| {
            System::set_block_number(1);
            // no key was ever minted: nothing can be proposed by anyone
            assert_noop!(
                Multisig::propose_upgrade(RuntimeOrigin::signed(1), [1u8; 32], sealed()),
                Error::<Test>::NotSigner
            );
        });
    }
}
