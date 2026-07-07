//! Harlequin justice pallet — disputes judged by a **jury drawn by reputation-weighted VRF sortition**
//! (SPEC §4), with **robust interest-exclusion** (Art. IX: *"nadie es juez… de aquella en que tenga
//! interés"*). It is the on-chain execution of the constitutional backstop (pre-freeze audit, issue #5):
//! the sealed canon (Art. XII) only bites if the jury that applies it is independent of the parties — so
//! the load-bearing part of this pallet is **who is kept off the jury**, not the tally.
//!
//! ## What it is — and what it deliberately is NOT
//! - A verdict **certifies a fact and makes it public** (Art. IX/X) and triggers the *reputational*
//!   consequence (slashing, §1.7) through a pluggable handler. That is the ONLY consequence the protocol
//!   automates.
//! - It **never wields force** (Art. I, line red in SPEC §4b.2): no seizure, no freezing, no coerced
//!   restitution. From there it is free people who, associating or shunning, give each name its measure.
//!   This pallet stops at "the fact is proven and the standing falls".
//!
//! ## The three separated functions (SPEC §4) and where this sits
//! This is the **judicial** function only. Selection is by VRF sortition over reputation — *not nameable*
//! by any operator — so the separation of powers is **coded, not trusted**. There is no permanent court
//! and **precedent does not bind** (Art. XII): each jury is freshly drawn (§4e is a future increment that
//! tags a case "interpretive" and binds that same drawn jury to the canon).
//!
//! ## Interest-exclusion, enforced in THREE places (defence in depth)
//! "Interest" is **substantive**, sourced from the trust graph through [`InterestInspect`] (a party, or
//! vouch-related to a party, or any account explicitly flagged interested — e.g. the rule-maker whose
//! rule is being challenged, §5). It is enforced:
//! 1. **At the draw** — interested accounts are removed from the candidate pool, so they cannot be picked.
//! 2. **At the vote** — a juror that is interested is rejected even if it was drawn (state can change).
//! 3. **At the tally** — any recorded vote from an interested account is discarded before counting.
//! A weak "interest" check is exactly the failure the audit flagged; hence the redundancy.
//!
//! ## Honest limitations (pre-mainnet)
//! - **Jury draw is computed centrally and deterministically** (reuses the cross-validated
//!   `consensus-core` sortition over the reputation snapshot, seeded by case + block) → verify-by-recompute.
//!   The production form is per-juror **VRF self-election with a proof**; the trait/seed seam is ready for it.
//! - **Weights are placeholders, not benchmarked.**
//! - **Randomness** for the seed is the parent block hash; a manipulable beacon is a known weakness →
//!   move to a VRF/randomness beacon before mainnet.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

/// Reputation source: the per-account consensus reputation (the conservative MIN across the four suits,
/// SPEC §1.2b / §4 "piso = mín(todas)") that the jury draw is weighted by. Wired by the runtime to the
/// reputation pallet.
pub trait ReputationInspect<AccountId> {
    /// Every account with positive consensus reputation and its raw fixed-point weight.
    fn consensus_reputation() -> alloc::vec::Vec<(AccountId, i128)>;
}

/// Substantive interest test (Art. IX). `who` is interested in a case whose involved accounts are
/// `parties` if it is one of them OR is related to one (vouch edge in either direction, shared cluster,
/// …). Wired by the runtime to the reputation pallet's trust graph — keeping "interest" meaningful is
/// what makes the constitutional backstop bite (issue #5).
pub trait InterestInspect<AccountId> {
    fn has_interest(who: &AccountId, parties: &[AccountId]) -> bool;

    /// Relation depth from `who` to the nearest party in the vouch graph, or `None` if independent within
    /// the queried depth. depth-1 is already handled by [`has_interest`](Self::has_interest) (excluded at
    /// the draw); a `Some(2)` lets the draw **weight-penalise** a 2-hop ring juror (SPEC §4i-(8)) without
    /// a hard cut. Default `None` → no penalty, so existing wirings compile and behave unchanged.
    fn relation_depth(_who: &AccountId, _parties: &[AccountId]) -> Option<u32> {
        None
    }
}

/// What happens to a culprit found guilty: **rectification of the record** (Acta J2, Art. IX —
/// reputation is TESTIMONY; a verdict strikes a proven falsehood, it never fines). Wired by the runtime
/// to the reputation pallet's evidence journal. `dimension` is the suit index (0..3 = the four suits);
/// the runtime maps it to its `Suit`. The protocol automates **nothing beyond this**.
pub trait RectifyOnVerdict<AccountId> {
    /// Strike the case's named evidence records and apply the (mechanically capped) reputational loss:
    /// `applied = min(loss, current worth of the named records)` — gate E5. Returns what was applied
    /// (0 when the case named no live evidence — gate E6: the verdict then only marks).
    fn on_guilty(culprit: &AccountId, dimension: u8, loss: u128, evidence_ids: &[u64]) -> u128;

    /// Whether `id` is a LIVE evidence record of `who` in `dimension` (non-zero current worth). Used at
    /// `open_case` to reject foreign/struck/dust references EARLY, not silently zero them at verdict.
    fn owns_evidence(id: u64, who: &AccountId, dimension: u8) -> bool;
}

/// Un-grindable randomness source for the jury draw (macroaudit §2.1). Wired by the runtime to the
/// beacon pallet's commit–reveal beacon: the draw seed is unpredictable until after keys are committed,
/// and a candidate's sortition key is the secret it COMMITTED an epoch earlier (not a freely-chosen id),
/// so a member cannot grind its name (Art. VII lets one pick an id) against a known seed to lift its
/// seats. A candidate with no committed key this epoch is simply not eligible for the draw.
pub trait BeaconInspect<AccountId> {
    /// The current beacon seed (hex) for this epoch's draw.
    fn seed() -> alloc::string::String;
    /// The committed sortition key (hex) of `who` this epoch, or `None` if it did not commit+reveal.
    fn committed_key(who: &AccountId) -> Option<alloc::string::String>;
}

#[frame::pallet]
pub mod pallet {
    use super::{BeaconInspect, InterestInspect, RectifyOnVerdict, ReputationInspect};
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;
    use frame::prelude::*;

    /// State of a dispute on the public board (SPEC §4b.2).
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
    pub enum CaseStatus {
        /// Jury drawn, votes open.
        Open,
        /// Quorum reached: the fact is proven, the standing fell.
        ResolvedGuilty,
        /// Quorum not reached: "the mere accusation decides nothing" (Art. IX) — name intact.
        ResolvedNotGuilty,
        /// The extended voting window closed without a voting quorum (SPEC-RELAUNCH §7-🔴#4): NO verdict.
        /// Not a free NotGuilty — the fact was never judged and the dispute may be opened again. Lapsing
        /// exists so an unvoted case cannot hang over the defendant forever, without letting silence acquit.
        Lapsed,
    }

    /// A dispute. The contested fact lives off-chain; the chain holds only its hash (Art. X: the light is
    /// on the rules, not the lives; selective disclosure to the jury, §4d).
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
    )]
    pub struct Case<AccountId, BlockNumber> {
        /// Who opened the dispute.
        pub plaintiff: AccountId,
        /// The accused, who answers for the alleged fact.
        pub defendant: AccountId,
        /// Commitment to the contested fact (off-chain content).
        pub fact_hash: [u8; 32],
        /// Suit index (0..3) the alleged fraud pertains to — the dimension rectified on a guilty verdict.
        pub dimension: u8,
        /// Reputational loss REQUESTED if found guilty — mechanically capped at verdict by the current
        /// worth of `evidence_ids` (Acta J2, gate E5): a jury cannot take more than the falsehood proved.
        pub loss: u128,
        pub status: CaseStatus,
        /// Block the case was opened at (SPEC-RELAUNCH §7-🔴#4): anchors the voting window — a case can
        /// no longer be closed in the same breath it was opened (the v1 self-acquittal hole).
        pub opened_at: BlockNumber,
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Reputation source the jury draw is weighted by.
        type Reputation: ReputationInspect<Self::AccountId>;

        /// Substantive interest test (Art. IX) — the load-bearing exclusion.
        type Interest: InterestInspect<Self::AccountId>;

        /// Consequence of a guilty verdict: rectification of the named evidence records (Acta J2, §1.7).
        type Rectifier: RectifyOnVerdict<Self::AccountId>;

        /// Max evidence-journal records one case may dispute (bounds the case and the rectification).
        #[pallet::constant]
        type MaxCaseEvidence: Get<u32>;

        /// Un-grindable randomness for the draw (§2.1) — the commit–reveal beacon (beacon pallet).
        type Beacon: BeaconInspect<Self::AccountId>;

        /// `τ` for the sortition: the expected jury size (SPEC §2.2 / §4 rotating panel).
        #[pallet::constant]
        type JurySize: Get<u32>;

        /// Hard cap on jurors stored per case (bounds storage / PoV).
        #[pallet::constant]
        type MaxJury: Get<u32>;

        /// Max accounts explicitly flagged interested when opening a case (bounds the exclusion list).
        #[pallet::constant]
        type MaxInterested: Get<u32>;

        /// Supermajority of the DRAWN jury required to find guilt (SPEC §4: "supermayoría honesta"). The
        /// fraction is of the whole jury, not only of those who voted — silence is not guilt.
        #[pallet::constant]
        type GuiltThreshold: Get<Permill>;

        /// Minimum blocks a case must stay open before it can be closed (SPEC-RELAUNCH §7-🔴#4:
        /// ~1 day wall-clock). Kills the v1 hole where the accused closed with 0 votes in the same block.
        #[pallet::constant]
        type VotingWindow: Get<BlockNumberFor<Self>>;

        /// Fraction of the DRAWN jury that must have VOTED (either way) for a close to produce a verdict
        /// (SPEC-RELAUNCH §7-🔴#4, default 2/3). Below it: the close is refused until the extended window
        /// (2× `VotingWindow`) runs out, after which the case LAPSES without verdict.
        #[pallet::constant]
        type VoteQuorum: Get<Permill>;

        /// Sortition-weight multiplier applied to a **depth-2-related** candidate at the jury draw (SPEC
        /// §4i-(8), value quantified by simulation: ≈10% → `Permill::from_percent(10)` drops
        /// P(2-hop ring blocks a verdict) below ~1.5% up to a 80-strong loyal ring, WITHOUT a hard cut, so
        /// the pool is not shrunk and pool-poisoning is not weaponised). depth-1 is excluded outright; this
        /// only down-weights the softer 2-hop tie. `from_percent(100)` disables the penalty.
        #[pallet::constant]
        type Depth2WeightPermille: Get<Permill>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Next case id.
    #[pallet::storage]
    pub type NextCaseId<T> = StorageValue<_, u64, ValueQuery>;

    /// All cases by id (the public board, SPEC §4b.2).
    #[pallet::storage]
    pub type Cases<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, Case<T::AccountId, BlockNumberFor<T>>, OptionQuery>;

    /// (Acta J2) The CONCRETE evidence-journal records each case disputes — validated at open (live,
    /// the defendant's, right suit; deduped). A guilty verdict rectifies EXACTLY these and nothing
    /// else; empty (e.g. incapacity cases) → a guilty verdict applies no loss, only a mark. Kept beside
    /// `Jury`/`Parties` rather than inside `Case` (same bounded-per-case pattern).
    #[pallet::storage]
    pub type CaseEvidence<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, BoundedVec<u64, T::MaxCaseEvidence>, ValueQuery>;

    /// (Acta J2 c) The PUBLIC mark of guilt: account → case ids it was found guilty in. THIS is what a
    /// verdict always leaves — even when no evidence could be rectified (gate E6) the fact is certified
    /// "under verdict X" on-chain, queryable by anyone deciding whether to deal with the name (Art.
    /// IX/X: the verdict publishes; free people weigh it). Bounded: past [`MAX_GUILTY_MARKS`] the OLDEST
    /// mark is dropped — the full trail stays recomputable from `CaseResolved` events forever.
    #[pallet::storage]
    pub type GuiltyMarks<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        BoundedVec<u64, ConstU32<MAX_GUILTY_MARKS>>,
        ValueQuery,
    >;

    /// Bound of [`GuiltyMarks`] per account (a name with 64 guilty verdicts has said all it needs to).
    pub const MAX_GUILTY_MARKS: u32 = 64;

    /// Case ids that are incapacity challenges (§1.5e, Art. III): a guilty verdict NULLIFIES the contested
    /// consent rather than slashing. Absence = ordinary fraud case (no-regression).
    #[pallet::storage]
    pub type IncapacityCase<T: Config> = StorageMap<_, Blake2_128Concat, u64, (), OptionQuery>;

    /// The drawn jury per case (pseudonyms are public, SPEC §4b.2).
    #[pallet::storage]
    pub type Jury<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, BoundedVec<T::AccountId, T::MaxJury>, ValueQuery>;

    /// The full set of interested accounts per case (parties + explicit flags), kept so the vote/tally
    /// guards re-check interest against the SAME parties the draw used.
    #[pallet::storage]
    pub type Parties<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, BoundedVec<T::AccountId, T::MaxInterested>, ValueQuery>;

    /// Each juror's vote per case: `true` = guilty.
    #[pallet::storage]
    pub type Votes<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat,
        u64,
        Blake2_128Concat,
        T::AccountId,
        bool,
        OptionQuery,
    >;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A dispute was opened and its jury drawn (interest-excluded).
        CaseOpened { case: u64, plaintiff: T::AccountId, defendant: T::AccountId, jurors: u32 },
        /// A juror cast a vote.
        VoteCast { case: u64, juror: T::AccountId, guilty: bool },
        /// A case was resolved; `guilty` carries the verdict. `rectified` is the loss ACTUALLY applied
        /// (Acta J2: `min(requested, worth of the case's evidence)` — E5 verifiable from events alone).
        CaseResolved { case: u64, guilty: bool, guilty_votes: u32, jury: u32, rectified: u128 },
        /// (Acta J2 c) A guilty verdict marked `who` on-chain under case `case` — the certification a
        /// verdict ALWAYS leaves, even when it rectified nothing (gate E6).
        MarkedUnderVerdict { who: T::AccountId, case: u64 },
        /// The extended window ran out without a voting quorum: no verdict, reopenable (§7-🔴#4).
        CaseLapsed { case: u64, votes_cast: u32, jury: u32 },
        /// A guilty incapacity verdict (§1.5e, Art. III): the consent attributed to `subject` is nullified.
        /// The ONLY effect — no slash, no movement of keys/funds, no guardianship (escudo-no-espada).
        ConsentNullified { case: u64, subject: T::AccountId },
        /// Accountability signal (SPEC §4i-(8)(c)): SEATED jurors depth-2-related to a party, as triples
        /// `(juror, related_party, distance)` captured AT THE DRAW. NOT a punishment and NOT an exclusion
        /// — they were down-weighted, not cut. Carrying the related party + distance makes the cross-case
        /// pattern (a juror that recurs voting for its relation) **recomputable from events alone**, with
        /// no replay of the historical vouch graph. The verdict-worthy signal is the repeated
        /// pattern over many cases — inherently off-chain (or a future juror-reputation pallet), never a
        /// single case. Empty → not emitted.
        /// Each tuple is `(juror, related_party, distance, penalised_weight)`: the 4th field is the juror's
        /// sortition weight AFTER the depth-2 penalty (`Depth2WeightPermille`), so a validator on a small
        /// deterministic devnet — where the seated set alone cannot reveal the down-weight — reads the
        /// applied factor straight from the event (`penalised_weight == base_rep · Depth2WeightPermille`).
        Depth2JurorsSeated { case: u64, relations: Vec<(T::AccountId, T::AccountId, u32, i128)> },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Nobody sues themselves.
        SelfCase,
        /// No reputable, non-interested accounts were available to form a jury.
        EmptyJury,
        /// Too many explicitly-flagged interested accounts.
        TooManyInterested,
        /// The case does not exist.
        UnknownCase,
        /// The case is not open for votes / resolution.
        NotOpen,
        /// The caller was not drawn for this jury.
        NotAJuror,
        /// The caller has an interest in this case and cannot judge it (Art. IX).
        Interested,
        /// The voting window has not elapsed yet (SPEC-RELAUNCH §7-🔴#4).
        VotingStillOpen,
        /// Not enough of the jury has voted and the extended window has not run out yet.
        QuorumNotReached,
        /// A party to the case cannot close it before the extended window is exhausted.
        PartyCannotClose,
        /// (Acta J2) A named evidence record does not exist, is not the defendant's, is in another
        /// suit, or is already struck/dust — rejected at open, not silently zeroed at verdict.
        ForeignEvidence,
        /// Too many evidence records named in one case.
        TooManyEvidenceRefs,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Open a dispute against `defendant` over the fact committed to by `fact_hash`, proposing a
        /// reputational `loss` if proven. `evidence_ids` names the CONCRETE evidence-journal records the
        /// case disputes (Acta J2): each must be live, the defendant's, and in `dimension` — validated
        /// HERE, so a case built on foreign or dead evidence dies at the door instead of squatting a
        /// whole voting window. A guilty verdict rectifies exactly those records, capped by their worth.
        /// `extra_interested` flags accounts that must ALSO be kept off the
        /// jury (e.g. the maker/beneficiary of a rule under an interpretive challenge, §5). The jury is
        /// drawn immediately by reputation-weighted sortition, **excluding every interested account**.
        #[pallet::call_index(0)]
        // Honest manual upper bound (B1): the jury draw walks the FULL reputable
        // population × parties (`do_open` → `draw_jury`), unbounded by the call's byte length.
        // 7.5×10⁹ ref_time (7.5 ms) → `CheckWeight` admits ~100/block against the 750 ms normal
        // budget: CPU-DoS dies at admission. Real benchmark = post-launch.
        #[pallet::weight(Weight::from_parts(7_500_000_000, 0))]
        pub fn open_case(
            origin: OriginFor<T>,
            defendant: T::AccountId,
            fact_hash: [u8; 32],
            dimension: u8,
            loss: u128,
            evidence_ids: Vec<u64>,
            extra_interested: Vec<T::AccountId>,
        ) -> DispatchResult {
            let plaintiff = ensure_signed(origin)?;
            Self::do_open(
                plaintiff,
                defendant,
                fact_hash,
                dimension,
                loss,
                evidence_ids,
                extra_interested,
                false,
            )
        }

        /// Open an **incapacity challenge** (§1.5e, Art. III): contest that `subject` validly consented to
        /// the fact committed by `fact_hash`, because they lacked the discernment to consent. It is judged
        /// by the SAME sortitioned, interest-excluded jury as any dispute — capacity is **adjudicated, never
        /// decreed by an authority** (presumption of capacity; the challenger carries the burden). A guilty
        /// verdict NULLIFIES the attributed consent and nothing more: it never slashes, never moves the
        /// subject's keys or property, and never appoints a guardian (escudo-no-espada).
        #[pallet::call_index(3)]
        // Same jury draw as `open_case` → same honest upper bound (B1).
        #[pallet::weight(Weight::from_parts(7_500_000_000, 0))]
        pub fn open_incapacity_case(
            origin: OriginFor<T>,
            subject: T::AccountId,
            fact_hash: [u8; 32],
            extra_interested: Vec<T::AccountId>,
        ) -> DispatchResult {
            let plaintiff = ensure_signed(origin)?;
            // dimension = judicial (2) for the accountability trail; loss = 0 and NO evidence records
            // (incapacity never rectifies — its guilty verdict only nullifies the attributed consent).
            Self::do_open(plaintiff, subject, fact_hash, 2, 0, Vec::new(), extra_interested, true)
        }

        /// Cast a juror's vote. Rejected unless the caller was drawn AND has no interest now (Art. IX,
        /// second guard — interest can arise after the draw).
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(100_000, 0))]
        pub fn cast_vote(origin: OriginFor<T>, case: u64, guilty: bool) -> DispatchResult {
            let juror = ensure_signed(origin)?;
            let c = Cases::<T>::get(case).ok_or(Error::<T>::UnknownCase)?;
            ensure!(c.status == CaseStatus::Open, Error::<T>::NotOpen);
            ensure!(Jury::<T>::get(case).contains(&juror), Error::<T>::NotAJuror);
            let parties = Parties::<T>::get(case);
            ensure!(!T::Interest::has_interest(&juror, &parties), Error::<T>::Interested);
            Votes::<T>::insert(case, &juror, guilty);
            Self::deposit_event(Event::VoteCast { case, juror, guilty });
            Ok(())
        }

        /// Resolve a case: tally the drawn jury's guilty votes (discarding any from accounts now found
        /// interested — third guard) and compare against the supermajority threshold over the FULL jury.
        /// Guilty → certify the fact and apply the reputational loss (§1.7). Anyone may close it —
        /// EXCEPT a party, until the extended window runs out (SPEC-RELAUNCH §7-🔴#4: no self-acquittal).
        ///
        /// Window/quorum rules (§7-🔴#4): no close before `VotingWindow` blocks have passed; a close
        /// between 1× and 2× the window needs `VoteQuorum` of the jury to have voted; past 2× the window
        /// anyone may close with whatever votes exist — and if the quorum was never reached the case
        /// LAPSES without verdict (reopenable; silence neither convicts nor acquits).
        #[pallet::call_index(2)]
        // Honest manual upper bound (B1): verdict application walks the vouch graph for
        // clawback (O(vouch edges)). 3.75×10⁹ ref_time → ~200/block. NOTE (the reviewer, acta): these fixed
        // weights are sized for the LAUNCH population; the real cost grows with the reputation/vouch
        // graph — re-benchmark BEFORE the graph grows (F5+), or admission protection erodes.
        #[pallet::weight(Weight::from_parts(3_750_000_000, 0))]
        pub fn close_case(origin: OriginFor<T>, case: u64) -> DispatchResult {
            let closer = ensure_signed(origin)?;
            let mut c = Cases::<T>::get(case).ok_or(Error::<T>::UnknownCase)?;
            ensure!(c.status == CaseStatus::Open, Error::<T>::NotOpen);

            let now = frame_system::Pallet::<T>::block_number();
            let window = T::VotingWindow::get();
            let elapsed = now.saturating_sub(c.opened_at);
            ensure!(elapsed >= window, Error::<T>::VotingStillOpen);
            let extended_over = elapsed >= window.saturating_add(window);

            let jury = Jury::<T>::get(case);
            let parties = Parties::<T>::get(case);
            // A party may only close once the extended window is exhausted (and then only into a tally
            // or a lapse that a full jury quorum did not produce — never a same-block self-acquittal).
            if !extended_over {
                ensure!(!parties.contains(&closer), Error::<T>::PartyCannotClose);
            }
            let jury_n = jury.len() as u32;
            let mut guilty_votes: u32 = 0;
            let mut votes_cast: u32 = 0;
            for juror in jury.iter() {
                // Third guard: an interested account's vote never counts, even if it slipped through.
                if T::Interest::has_interest(juror, &parties) {
                    continue;
                }
                match Votes::<T>::get(case, juror) {
                    Some(true) => {
                        votes_cast = votes_cast.saturating_add(1);
                        guilty_votes = guilty_votes.saturating_add(1);
                    }
                    Some(false) => votes_cast = votes_cast.saturating_add(1),
                    None => {}
                }
            }
            let quorum = T::VoteQuorum::get().mul_ceil(jury_n);
            if votes_cast < quorum {
                if !extended_over {
                    // Not enough of the jury has spoken and there is still time: the close waits.
                    return Err(Error::<T>::QuorumNotReached.into());
                }
                // Extended window exhausted without quorum: the case LAPSES — no verdict, no free
                // NotGuilty, the dispute may be opened anew (SPEC-RELAUNCH §7-🔴#4).
                c.status = CaseStatus::Lapsed;
                Cases::<T>::insert(case, c);
                Self::deposit_event(Event::CaseLapsed { case, votes_cast, jury: jury_n });
                return Ok(());
            }
            // Guilty needs a supermajority of the WHOLE jury (silence is not guilt).
            let needed = T::GuiltThreshold::get().mul_ceil(jury_n);
            let guilty = guilty_votes >= needed && needed > 0;

            c.status = if guilty { CaseStatus::ResolvedGuilty } else { CaseStatus::ResolvedNotGuilty };
            let defendant = c.defendant.clone();
            let dimension = c.dimension;
            let loss = c.loss;
            let evidence_ids = CaseEvidence::<T>::get(case);
            Cases::<T>::insert(case, c);

            let mut rectified: u128 = 0;
            if guilty {
                if IncapacityCase::<T>::contains_key(case) {
                    // §1.5e (Art. III): the sole effect is to nullify the falsely-attributed consent —
                    // no slash, no movement of the subject's keys or property, no guardian. Escudo, no
                    // espada. NO mark either: the subject did nothing wrong — a consent was voided.
                    Self::deposit_event(Event::ConsentNullified { case, subject: defendant.clone() });
                } else {
                    // Ordinary fraud (Acta J2): rectify EXACTLY the records the case named, capped
                    // mechanically at their current worth (E5); a case with no live evidence rectifies
                    // nothing and the standing is untouched (E6). No force beyond this (Art. I).
                    rectified =
                        T::Rectifier::on_guilty(&defendant, dimension, loss, &evidence_ids);
                    // The verdict ALWAYS marks (Acta J2 c): the fact is certified on-chain under this
                    // case id, whatever the rectification amounted to.
                    GuiltyMarks::<T>::mutate(&defendant, |marks| {
                        if marks.is_full() {
                            marks.remove(0);
                        }
                        let _ = marks.try_push(case);
                    });
                    Self::deposit_event(Event::MarkedUnderVerdict {
                        who: defendant.clone(),
                        case,
                    });
                }
            }
            Self::deposit_event(Event::CaseResolved {
                case,
                guilty,
                guilty_votes,
                jury: jury_n,
                rectified,
            });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Shared logic to open a dispute. `incapacity = true` marks the case so its guilty verdict
        /// nullifies the contested consent (§1.5e, Art. III) instead of slashing (§1.7) — both share the
        /// SAME sortitioned, interest-excluded jury, so capacity is adjudicated by peers, never decreed.
        fn do_open(
            plaintiff: T::AccountId,
            defendant: T::AccountId,
            fact_hash: [u8; 32],
            dimension: u8,
            loss: u128,
            evidence_ids: Vec<u64>,
            extra_interested: Vec<T::AccountId>,
            incapacity: bool,
        ) -> DispatchResult {
            ensure!(plaintiff != defendant, Error::<T>::SelfCase);

            // (Acta J2, the reviewer's review pt. 2) Validate the disputed evidence AT THE DOOR: every named
            // record must be a live record of the DEFENDANT in THIS dimension. Dedup so a repeated id
            // cannot pad the case. Rejecting here keeps a junk case from looking valid all window long.
            let evidence_ids: alloc::collections::BTreeSet<u64> = evidence_ids.into_iter().collect();
            for id in evidence_ids.iter() {
                ensure!(
                    T::Rectifier::owns_evidence(*id, &defendant, dimension),
                    Error::<T>::ForeignEvidence
                );
            }
            let evidence_bv: BoundedVec<u64, T::MaxCaseEvidence> = evidence_ids
                .into_iter()
                .collect::<Vec<u64>>()
                .try_into()
                .map_err(|_| Error::<T>::TooManyEvidenceRefs)?;

            // Parties = plaintiff + defendant + explicit flags. The interest test widens this through the
            // trust graph; this is the seed set both the draw and the later guards re-use.
            let mut parties: Vec<T::AccountId> = alloc::vec![plaintiff.clone(), defendant.clone()];
            parties.extend(extra_interested.into_iter());
            let parties_bv: BoundedVec<T::AccountId, T::MaxInterested> =
                parties.clone().try_into().map_err(|_| Error::<T>::TooManyInterested)?;

            let case_id = NextCaseId::<T>::get();
            let jurors = Self::draw_jury(case_id, &parties);
            ensure!(!jurors.is_empty(), Error::<T>::EmptyJury);
            let jury_bv: BoundedVec<T::AccountId, T::MaxJury> =
                jurors.try_into().unwrap_or_default();
            ensure!(!jury_bv.is_empty(), Error::<T>::EmptyJury);

            Cases::<T>::insert(
                case_id,
                Case {
                    plaintiff: plaintiff.clone(),
                    defendant: defendant.clone(),
                    fact_hash,
                    dimension,
                    loss,
                    status: CaseStatus::Open,
                    opened_at: frame_system::Pallet::<T>::block_number(),
                },
            );
            CaseEvidence::<T>::insert(case_id, evidence_bv);
            // §1.5e: mark incapacity cases so close_case nullifies the attributed consent instead of
            // slashing. Absence of the marker = ordinary fraud case (no-regression).
            if incapacity {
                IncapacityCase::<T>::insert(case_id, ());
            }
            let jurors_n = jury_bv.len() as u32;
            // Accountability trail (SPEC §4i-(8)(c)): for each SEATED juror, record (juror, the party it is
            // depth-2-related to, the distance, the post-penalty sortition weight) — captured here at the
            // draw so the cross-case pattern AND the applied down-weight are recomputable from events alone
            // (no vouch-graph replay). depth-1 jurors are already excluded, so a relation here is
            // depth ≥ 2. One tuple per juror (the first related party).
            let permille = T::Depth2WeightPermille::get();
            let reps = T::Reputation::consensus_reputation();
            let depth2: Vec<(T::AccountId, T::AccountId, u32, i128)> = jury_bv
                .iter()
                .filter_map(|j| {
                    parties.iter().find_map(|p| {
                        match T::Interest::relation_depth(j, core::slice::from_ref(p)) {
                            Some(d) if d >= 2 => {
                                let base = reps
                                    .iter()
                                    .find(|(a, _)| a == j)
                                    .map(|(_, r)| *r)
                                    .unwrap_or(0);
                                let w = permille.mul_floor(base.max(0) as u128) as i128;
                                Some((j.clone(), p.clone(), d, w))
                            }
                            _ => None,
                        }
                    })
                })
                .collect();
            Jury::<T>::insert(case_id, jury_bv);
            Parties::<T>::insert(case_id, parties_bv);
            NextCaseId::<T>::put(case_id.saturating_add(1));

            Self::deposit_event(Event::CaseOpened {
                case: case_id,
                plaintiff,
                defendant,
                jurors: jurors_n,
            });
            if !depth2.is_empty() {
                Self::deposit_event(Event::Depth2JurorsSeated { case: case_id, relations: depth2 });
            }
            Ok(())
        }

        /// Draw the jury for `case_id`: reputation-weighted VRF sortition (cross-validated `consensus-core`)
        /// over every reputable account **minus the interested ones** (Art. IX). Deterministic and
        /// verify-by-recompute: the seed binds the case id and the parent block hash. Returns the drawn
        /// accounts (those that won ≥1 seat), capped to `MaxJury`, ordered deterministically.
        fn draw_jury(case_id: u64, parties: &[T::AccountId]) -> Vec<T::AccountId> {
            use alloc::collections::BTreeMap;
            // 1. eligible pool: reputable AND not interested. The exclusion is by CONSTRUCTION — an
            //    interested account is never even a candidate.
            //    depth-2-related candidates are NOT excluded (a hard cut shrinks the pool ~35% and opens
            //    pool-poisoning) but **down-weighted** by `Depth2WeightPermille` (SPEC §4i-(8)) — pure
            //    integer Permill math, no float (runtime determinism).
            let depth2_weight = T::Depth2WeightPermille::get();
            let pool: Vec<(T::AccountId, i128)> = T::Reputation::consensus_reputation()
                .into_iter()
                .filter(|(acc, rep)| *rep > 0 && !T::Interest::has_interest(acc, parties))
                .map(|(acc, rep)| {
                    let w = if T::Interest::relation_depth(&acc, parties) == Some(2) {
                        depth2_weight.mul_floor(rep as u128) as i128
                    } else {
                        rep
                    };
                    (acc, w)
                })
                .filter(|(_, w)| *w > 0) // a penalty can floor a tiny weight to 0 → drops out (negligible)
                .collect();
            if pool.is_empty() {
                return Vec::new();
            }
            // 2. key accounts by index string (consensus-core keys agents by short string ids). The
            //    sortition key is each candidate's COMMITTED beacon key (macroaudit §2.1) — NOT a
            //    freely-chosen id — so the draw cannot be ground. A candidate that did not commit+reveal
            //    this epoch has no key and is not eligible.
            let mut reputation: BTreeMap<String, i128> = BTreeMap::new();
            let mut secret_keys: BTreeMap<String, String> = BTreeMap::new();
            let mut eligible: Vec<T::AccountId> = Vec::new();
            for (acc, rep) in pool.into_iter() {
                if let Some(key) = T::Beacon::committed_key(&acc) {
                    let id = eligible.len().to_string();
                    reputation.insert(id.clone(), rep);
                    secret_keys.insert(id, key);
                    eligible.push(acc);
                }
            }
            if eligible.is_empty() {
                return Vec::new();
            }
            // 3. seed = the un-grindable commit–reveal beacon, bound to the case id (per-case draw,
            //    deterministic to verify). Unpredictable until after keys were committed → no grinding.
            let seed = alloc::format!("hlq-jury-{}-{}", case_id, T::Beacon::seed());

            // 4. run the sortition; jurors = candidates that won at least one seat.
            // LIVENESS/SAFETY FLOOR (J-HIGH2, audit): mirror the finality committee's size floor
            // (sortition_fp.rs:262). The Poisson draw is size-blind and can seat a jury too small to resist a
            // single bribed/colluding juror — a jury of 1 makes GuiltThreshold.mul_ceil(1)=1, so ONE juror
            // decides the verdict. Floor = min(4, eligible): below it, re-draw with a salted seed
            // (deterministic — every verifier recomputes the same retries) keeping the LARGEST draw, up to 32
            // tries. Where the eligible pool is itself < 4, the jury is the whole pool (can't do better).
            let cap = T::MaxJury::get() as usize;
            let jury_size = T::JurySize::get();
            let tally = |s: &str| -> Vec<(usize, u32)> {
                let seats = consensus_core::sortition_fp::elect_committee_fp(
                    &reputation,
                    &secret_keys,
                    s,
                    jury_size,
                );
                // ordered by (seats desc, index asc) for determinism, capped at MaxJury.
                let mut won: Vec<(usize, u32)> = seats
                    .into_iter()
                    .filter_map(|(id, s)| if s >= 1 { id.parse::<usize>().ok().map(|i| (i, s)) } else { None })
                    .collect();
                won.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                won.truncate(cap);
                won
            };
            let floor = core::cmp::min(4u32, eligible.len() as u32);
            let mut best = tally(&seed);
            let mut retry = 0u32;
            while (best.len() as u32) < floor && retry < 32 {
                retry += 1;
                best = {
                    let redraw = tally(&alloc::format!("{seed}-retry-{retry}"));
                    if redraw.len() > best.len() { redraw } else { best }
                };
            }
            // 5. map indices back to accounts.
            best.into_iter().filter_map(|(i, _)| eligible.get(i).cloned()).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_justice;
    use crate::{BeaconInspect, CaseStatus, Cases, InterestInspect, RectifyOnVerdict, ReputationInspect};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Justice: pallet_justice,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    // --- Mock providers -----------------------------------------------------------------------------
    // A fixed reputable population (accounts 10..=25, all equally reputable) so the draw has a real pool.
    pub struct MockRep;
    impl ReputationInspect<u64> for MockRep {
        fn consensus_reputation() -> Vec<(u64, i128)> {
            (10u64..=25u64).map(|a| (a, 1_000i128)).collect()
        }
    }

    // Interest = being a party, OR being "kin" of a party. Kin relation: accounts whose ids differ by
    // exactly 100 are related (lets a test inject a vouch-style relation: party 12 ↔ kin 112). This
    // stands in for the trust-graph relation the runtime wires.
    // Accounts the test declares depth-2-related to a party (the trust-graph 2-hop ring the runtime wires
    // via `pallet_reputation::relation_depth`). Empty by default → `relation_depth` returns None → the
    // back-compat path (no penalty, no accountability event).
    parameter_types! {
        pub storage Depth2Set: Vec<u64> = Vec::new();
    }
    pub struct MockInterest;
    impl InterestInspect<u64> for MockInterest {
        fn has_interest(who: &u64, parties: &[u64]) -> bool {
            parties.iter().any(|p| p == who || p.abs_diff(*who) == 100)
        }
        fn relation_depth(who: &u64, parties: &[u64]) -> Option<u32> {
            if Self::has_interest(who, parties) {
                return Some(1);
            }
            if Depth2Set::get().contains(who) {
                return Some(2);
            }
            None
        }
    }

    // A mock evidence journal `(id, owner, dimension, worth)` + a log of applied rectifications, so
    // tests can assert the verdict's only consequence fired — mechanically capped (Acta J2).
    parameter_types! {
        pub storage SlashLog: Vec<(u64, u8, u128)> = Vec::new();
        pub storage MockEvidence: Vec<(u64, u64, u8, u128)> = Vec::new();
    }
    pub struct MockRectifier;
    impl RectifyOnVerdict<u64> for MockRectifier {
        fn on_guilty(culprit: &u64, dimension: u8, loss: u128, evidence_ids: &[u64]) -> u128 {
            let ev = MockEvidence::get();
            let cap: u128 = evidence_ids
                .iter()
                .filter_map(|id| {
                    ev.iter()
                        .find(|(i, o, d, _)| i == id && o == culprit && *d == dimension)
                        .map(|(_, _, _, w)| *w)
                })
                .sum();
            let applied = loss.min(cap);
            let mut v = SlashLog::get();
            v.push((*culprit, dimension, applied));
            SlashLog::set(&v);
            applied
        }
        fn owns_evidence(id: u64, who: &u64, dimension: u8) -> bool {
            MockEvidence::get()
                .iter()
                .any(|(i, o, d, w)| *i == id && o == who && *d == dimension && *w > 0)
        }
    }

    // Beacon stub: every account has a deterministic committed key (so the pool is unchanged from the
    // pre-beacon draw) and the seed is fixed. The grinding-resistance itself is proven in
    // `consensus-core::beacon`; here we only check the draw consumes the beacon seam correctly.
    pub struct MockBeacon;
    impl BeaconInspect<u64> for MockBeacon {
        fn seed() -> String {
            "mock-beacon-seed".to_string()
        }
        fn committed_key(who: &u64) -> Option<String> {
            Some(alloc::format!("sk-{who}"))
        }
    }

    parameter_types! {
        pub const JurySize: u32 = 12;
        pub const MaxJury: u32 = 32;
        pub const MaxInterested: u32 = 16;
        // 2/3 supermajority of the whole jury to find guilt.
        pub const GuiltThreshold: Permill = Permill::from_percent(67);
        // depth-2 jurors draw at 10% weight (SPEC §4i-(8), the quantified factor).
        pub const Depth2WeightPermille: Permill = Permill::from_percent(10);
        // §7-🔴#4: short window for tests — no close before block opened_at+5; lapse from opened_at+10.
        pub const VotingWindow: u64 = 5;
        // 2/3 of the jury must have voted for a close to produce a verdict.
        pub const VoteQuorum: Permill = Permill::from_percent(67);
    }

    impl pallet_justice::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type Reputation = MockRep;
        type Interest = MockInterest;
        type Rectifier = MockRectifier;
        type MaxCaseEvidence = ConstU32<16>;
        type Beacon = MockBeacon;
        type JurySize = JurySize;
        type MaxJury = MaxJury;
        type MaxInterested = MaxInterested;
        type GuiltThreshold = GuiltThreshold;
        type Depth2WeightPermille = Depth2WeightPermille;
        type VotingWindow = VotingWindow;
        type VoteQuorum = VoteQuorum;
    }

    /// Fast-forward past the voting window so a legacy-style close is allowed (quorum still applies).
    fn past_window() {
        System::set_block_number(System::block_number() + 6);
    }
    /// Fast-forward past the EXTENDED window (2×): closes tally with whatever votes exist, or lapse.
    fn past_extended_window() {
        System::set_block_number(System::block_number() + 11);
    }

    fn new_test_ext() -> TestState {
        let mut ext: TestState =
            frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into();
        ext.execute_with(|| System::set_block_number(1));
        ext
    }

    fn jury_of(case: u64) -> Vec<u64> {
        crate::Jury::<Test>::get(case).into_inner()
    }

    #[test]
    fn parties_are_never_drawn_on_the_jury() {
        new_test_ext().execute_with(|| {
            // plaintiff 10 vs defendant 11; both are in the reputable pool but must be excluded.
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 100, vec![], vec![]));
            let jury = jury_of(0);
            assert!(!jury.is_empty());
            assert!(!jury.contains(&10), "plaintiff judged its own case");
            assert!(!jury.contains(&11), "defendant judged its own case");
        });
    }

    // The (juror, related_party, distance, penalised_weight) tuples in the depth-2 event for `case`.
    fn depth2_event_relations(case: u64) -> Option<Vec<(u64, u64, u32, i128)>> {
        System::events().into_iter().find_map(|r| match r.event {
            RuntimeEvent::Justice(crate::Event::Depth2JurorsSeated { case: c, relations }) if c == case => {
                Some(relations)
            }
            _ => None,
        })
    }

    #[test]
    fn depth2_jurors_are_recorded_for_accountability() {
        new_test_ext().execute_with(|| {
            // The runtime would flag these from the vouch graph; here every eligible account is depth-2 to
            // a party, so the whole drawn jury must appear in the accountability trail (they were
            // down-weighted at the draw, NOT excluded — the penalty keeps them eligible, just lighter).
            Depth2Set::set(&(10u64..=25).collect::<Vec<_>>());
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 100, vec![], vec![]));
            let mut jury = jury_of(0);
            assert!(!jury.is_empty(), "penalty must not empty the jury — it down-weights, not cuts");
            let relations = depth2_event_relations(0).expect("accountability event not emitted");
            // one tuple per seated juror, carrying related party (first = plaintiff 10), distance 2, and the
            // post-penalty weight = base 1000 × 10% = 100 (proves the down-weight is applied, not just detected).
            let mut jurors_in_event: Vec<u64> = relations.iter().map(|(j, _, _, _)| *j).collect();
            jurors_in_event.sort();
            jury.sort();
            assert_eq!(jurors_in_event, jury, "every seated juror is depth-2 → one tuple each");
            assert!(
                relations.iter().all(|(_, party, dist, w)| *party == 10 && *dist == 2 && *w == 100),
                "each tuple carries party 10, distance 2, and penalised weight 1000×10% = 100"
            );
        });
    }

    #[test]
    fn no_depth2_event_when_no_two_hop_relation() {
        new_test_ext().execute_with(|| {
            // Depth2Set empty (default) → relation_depth returns None → back-compat path, no penalty.
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 100, vec![], vec![]));
            assert!(!jury_of(0).is_empty());
            assert!(depth2_event_relations(0).is_none(), "no 2-hop relation → no accountability event");
        });
    }

    #[test]
    fn kin_of_a_party_is_excluded_from_the_draw() {
        new_test_ext().execute_with(|| {
            // Make 12 a party; its "kin" is 112 — but 112 is not in the pool, so inject a pool kin: use
            // defendant 10 whose kin is 110 (not in pool). Instead test the substantive relation directly:
            // flag 13 explicitly interested; 13 must not appear, and neither may anything kin to parties.
            assert_ok!(Justice::open_case(
                RuntimeOrigin::signed(10),
                11,
                [0u8; 32],
                0,
                100,
                vec![],
                vec![13]
            ));
            let jury = jury_of(0);
            assert!(!jury.contains(&13), "explicitly-interested account was drawn");
            // every juror is non-interested w.r.t. the parties {10,11,13}
            for j in &jury {
                assert!(!MockInterest::has_interest(j, &[10, 11, 13]));
            }
        });
    }

    #[test]
    fn interested_account_cannot_vote() {
        new_test_ext().execute_with(|| {
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 100, vec![], vec![]));
            // a party (defendant) tries to vote — rejected (not a juror, and interested).
            assert_noop!(
                Justice::cast_vote(RuntimeOrigin::signed(11), 0, false),
                crate::Error::<Test>::NotAJuror
            );
        });
    }

    #[test]
    fn guilty_supermajority_rectifies_defendant() {
        new_test_ext().execute_with(|| {
            // the case names record 7: 11's evidence in dim 0, worth 250 — full rectification possible.
            MockEvidence::set(&vec![(7u64, 11u64, 0u8, 250u128)]);
            assert_ok!(Justice::open_case(
                RuntimeOrigin::signed(10),
                11,
                [0u8; 32],
                0,
                250,
                vec![7],
                vec![]
            ));
            let jury = jury_of(0);
            // all jurors vote guilty → well above 2/3.
            for j in &jury {
                assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(*j), 0, true));
            }
            // §7-🔴#4: the window must elapse, and a NON-party closes.
            past_window();
            assert_ok!(Justice::close_case(RuntimeOrigin::signed(jury[0]), 0));
            assert_eq!(Cases::<Test>::get(0).unwrap().status, CaseStatus::ResolvedGuilty);
            assert_eq!(SlashLog::get(), vec![(11u64, 0u8, 250u128)]);
            // the verdict marks the defendant on-chain (Acta J2 c)
            assert_eq!(crate::GuiltyMarks::<Test>::get(11u64).into_inner(), vec![0u64]);
        });
    }

    // --- Acta J2 (Art. IX): mechanical cap + mark — gates E5/E6 ---

    #[test]
    fn rectification_is_capped_by_the_case_evidence_e5() {
        new_test_ext().execute_with(|| {
            // the case's record is worth only 100 — a 250 request must apply exactly 100, mechanically.
            MockEvidence::set(&vec![(7u64, 11u64, 0u8, 100u128)]);
            assert_ok!(Justice::open_case(
                RuntimeOrigin::signed(10),
                11,
                [0u8; 32],
                0,
                250,
                vec![7],
                vec![]
            ));
            let jury = jury_of(0);
            for j in &jury {
                assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(*j), 0, true));
            }
            past_window();
            assert_ok!(Justice::close_case(RuntimeOrigin::signed(jury[0]), 0));
            assert_eq!(
                SlashLog::get(),
                vec![(11u64, 0u8, 100u128)],
                "E5: applied loss is min(requested, evidence worth)"
            );
        });
    }

    #[test]
    fn guilty_without_evidence_only_marks_e6() {
        new_test_ext().execute_with(|| {
            // a case naming NO evidence records: guilty certifies + marks, but rectifies NOTHING.
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 250, vec![], vec![]));
            let jury = jury_of(0);
            for j in &jury {
                assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(*j), 0, true));
            }
            past_window();
            assert_ok!(Justice::close_case(RuntimeOrigin::signed(jury[0]), 0));
            assert_eq!(Cases::<Test>::get(0).unwrap().status, CaseStatus::ResolvedGuilty);
            assert_eq!(SlashLog::get(), vec![(11u64, 0u8, 0u128)], "E6: zero applied, standing intact");
            assert_eq!(
                crate::GuiltyMarks::<Test>::get(11u64).into_inner(),
                vec![0u64],
                "E6: the verdict still marks"
            );
        });
    }

    #[test]
    fn foreign_or_dead_evidence_is_rejected_at_the_door() {
        new_test_ext().execute_with(|| {
            // record 7 belongs to account 12, not to defendant 11 → the case dies at open (early, not
            // silently zeroed at verdict — the reviewer's review pt. 2).
            MockEvidence::set(&vec![(7u64, 12u64, 0u8, 100u128)]);
            assert_noop!(
                Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 250, vec![7], vec![]),
                crate::Error::<Test>::ForeignEvidence
            );
            // right owner, wrong dimension → rejected too
            assert_noop!(
                Justice::open_case(RuntimeOrigin::signed(10), 12, [0u8; 32], 1, 250, vec![7], vec![]),
                crate::Error::<Test>::ForeignEvidence
            );
            // nonexistent record → rejected
            assert_noop!(
                Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 250, vec![99], vec![]),
                crate::Error::<Test>::ForeignEvidence
            );
        });
    }

    #[test]
    fn incapacity_verdict_nullifies_consent_without_slashing() {
        // §1.5e / Art. III (IC-1..5): a guilty incapacity verdict invalidates the attributed consent and
        // does NOTHING else — no slash, no key/fund movement, no guardian — and is reached by the SAME
        // sortitioned jury (adjudicated, never decreed by an authority).
        new_test_ext().execute_with(|| {
            assert_ok!(Justice::open_incapacity_case(RuntimeOrigin::signed(10), 11, [0u8; 32], vec![]));
            assert!(crate::IncapacityCase::<Test>::contains_key(0));
            let jury = jury_of(0);
            for j in &jury {
                assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(*j), 0, true));
            }
            past_window();
            assert_ok!(Justice::close_case(RuntimeOrigin::signed(jury[0]), 0));
            // verdict reached by the jury
            assert_eq!(Cases::<Test>::get(0).unwrap().status, CaseStatus::ResolvedGuilty);
            // and the ONLY effect is consent nullification — NO slash fired (escudo, no espada)
            assert!(SlashLog::get().is_empty());
        });
    }

    #[test]
    fn no_supermajority_means_not_guilty_and_no_slash() {
        new_test_ext().execute_with(|| {
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 250, vec![], vec![]));
            let jury = jury_of(0);
            // the whole jury votes (quorum met) but only ONE guilty → far below 2/3 → name intact,
            // no slash (Art. IX).
            assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(jury[0]), 0, true));
            for j in jury.iter().skip(1) {
                assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(*j), 0, false));
            }
            past_window();
            assert_ok!(Justice::close_case(RuntimeOrigin::signed(jury[1]), 0));
            assert_eq!(Cases::<Test>::get(0).unwrap().status, CaseStatus::ResolvedNotGuilty);
            assert!(SlashLog::get().is_empty());
        });
    }

    // --- §7-🔴#4 (SPEC-RELAUNCH): voting window + quorum + lapse — the v1 self-acquittal hole ---

    #[test]
    fn cannot_close_before_the_window() {
        new_test_ext().execute_with(|| {
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 250, vec![], vec![]));
            let jury = jury_of(0);
            for j in &jury {
                assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(*j), 0, true));
            }
            // same block, even with every vote in: the window must still elapse (no flash verdicts).
            assert_noop!(
                Justice::close_case(RuntimeOrigin::signed(jury[0]), 0),
                crate::Error::<Test>::VotingStillOpen
            );
        });
    }

    #[test]
    fn party_cannot_close_before_extended_window() {
        new_test_ext().execute_with(|| {
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 250, vec![], vec![]));
            let jury = jury_of(0);
            for j in &jury {
                assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(*j), 0, true));
            }
            past_window();
            // the DEFENDANT (and any party) cannot be the closer inside the extended window —
            // this is the exact v1 hole (accused closes on its own terms).
            assert_noop!(
                Justice::close_case(RuntimeOrigin::signed(11), 0),
                crate::Error::<Test>::PartyCannotClose
            );
        });
    }

    #[test]
    fn no_quorum_waits_then_lapses_without_verdict() {
        new_test_ext().execute_with(|| {
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 250, vec![], vec![]));
            let jury = jury_of(0);
            // only one juror ever votes → quorum (2/3 of the jury) never reached.
            assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(jury[0]), 0, true));
            past_window();
            assert_noop!(
                Justice::close_case(RuntimeOrigin::signed(jury[0]), 0),
                crate::Error::<Test>::QuorumNotReached
            );
            past_extended_window();
            // extended window exhausted: the case lapses — NO verdict, NO slash, NOT a NotGuilty.
            assert_ok!(Justice::close_case(RuntimeOrigin::signed(11), 0)); // even a party may close now
            assert_eq!(Cases::<Test>::get(0).unwrap().status, CaseStatus::Lapsed);
            assert!(SlashLog::get().is_empty());
            // and the dispute is reopenable — lapsing never acquits.
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 250, vec![], vec![]));
        });
    }

    #[test]
    fn nobody_sues_themselves() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                Justice::open_case(RuntimeOrigin::signed(10), 10, [0u8; 32], 0, 100, vec![], vec![]),
                crate::Error::<Test>::SelfCase
            );
        });
    }
}
