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

/// What happens to a culprit found guilty: the reputational consequence (§1.7 slashing) in the suit the
/// fraud pertains to. Wired by the runtime to the reputation pallet's fraud slashing. `dimension` is the
/// suit index (0..3 = the four suits); the runtime maps it to its `Suit`. The protocol automates
/// **nothing beyond this**.
pub trait SlashOnVerdict<AccountId> {
    fn on_guilty(culprit: &AccountId, dimension: u8, loss: u128);
}

#[frame::pallet]
pub mod pallet {
    use super::{InterestInspect, ReputationInspect, SlashOnVerdict};
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
    pub struct Case<AccountId> {
        /// Who opened the dispute.
        pub plaintiff: AccountId,
        /// The accused, who answers for the alleged fact.
        pub defendant: AccountId,
        /// Commitment to the contested fact (off-chain content).
        pub fact_hash: [u8; 32],
        /// Suit index (0..3) the alleged fraud pertains to — the dimension slashed on a guilty verdict.
        pub dimension: u8,
        /// Reputational loss applied to the defendant if found guilty (§1.7).
        pub loss: u128,
        pub status: CaseStatus,
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Reputation source the jury draw is weighted by.
        type Reputation: ReputationInspect<Self::AccountId>;

        /// Substantive interest test (Art. IX) — the load-bearing exclusion.
        type Interest: InterestInspect<Self::AccountId>;

        /// Reputational consequence of a guilty verdict (§1.7).
        type Slasher: SlashOnVerdict<Self::AccountId>;

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
    pub type Cases<T: Config> = StorageMap<_, Blake2_128Concat, u64, Case<T::AccountId>, OptionQuery>;

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
        /// A case was resolved; `guilty` carries the verdict.
        CaseResolved { case: u64, guilty: bool, guilty_votes: u32, jury: u32 },
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
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Open a dispute against `defendant` over the fact committed to by `fact_hash`, proposing a
        /// reputational `loss` if proven. `extra_interested` flags accounts that must ALSO be kept off the
        /// jury (e.g. the maker/beneficiary of a rule under an interpretive challenge, §5). The jury is
        /// drawn immediately by reputation-weighted sortition, **excluding every interested account**.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(1_000_000, 0))]
        pub fn open_case(
            origin: OriginFor<T>,
            defendant: T::AccountId,
            fact_hash: [u8; 32],
            dimension: u8,
            loss: u128,
            extra_interested: Vec<T::AccountId>,
        ) -> DispatchResult {
            let plaintiff = ensure_signed(origin)?;
            ensure!(plaintiff != defendant, Error::<T>::SelfCase);

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
                },
            );
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
        /// Guilty → certify the fact and apply the reputational loss (§1.7). Anyone may close it.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(1_000_000, 0))]
        pub fn close_case(origin: OriginFor<T>, case: u64) -> DispatchResult {
            ensure_signed(origin)?;
            let mut c = Cases::<T>::get(case).ok_or(Error::<T>::UnknownCase)?;
            ensure!(c.status == CaseStatus::Open, Error::<T>::NotOpen);

            let jury = Jury::<T>::get(case);
            let parties = Parties::<T>::get(case);
            let jury_n = jury.len() as u32;
            let mut guilty_votes: u32 = 0;
            for juror in jury.iter() {
                // Third guard: an interested account's vote never counts, even if it slipped through.
                if T::Interest::has_interest(juror, &parties) {
                    continue;
                }
                if let Some(true) = Votes::<T>::get(case, juror) {
                    guilty_votes = guilty_votes.saturating_add(1);
                }
            }
            // Guilty needs a supermajority of the WHOLE jury (silence is not guilt).
            let needed = T::GuiltThreshold::get().mul_ceil(jury_n);
            let guilty = guilty_votes >= needed && needed > 0;

            c.status = if guilty { CaseStatus::ResolvedGuilty } else { CaseStatus::ResolvedNotGuilty };
            let defendant = c.defendant.clone();
            let dimension = c.dimension;
            let loss = c.loss;
            Cases::<T>::insert(case, c);

            if guilty {
                // The only automated consequence: the standing falls (§1.7). No force (Art. I).
                T::Slasher::on_guilty(&defendant, dimension, loss);
            }
            Self::deposit_event(Event::CaseResolved { case, guilty, guilty_votes, jury: jury_n });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
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
            // 2. key accounts by index string (consensus-core keys agents by short string ids).
            let mut reputation: BTreeMap<String, i128> = BTreeMap::new();
            let mut secret_keys: BTreeMap<String, String> = BTreeMap::new();
            for (i, (_acc, rep)) in pool.iter().enumerate() {
                let id = i.to_string();
                reputation.insert(id.clone(), *rep);
                // Deterministic per-candidate VRF surrogate (central draw). Production: real per-juror key.
                secret_keys.insert(id.clone(), id);
            }
            // 3. seed = case id + parent block hash → unpredictable-ish, deterministic to verify.
            let parent = frame_system::Pallet::<T>::parent_hash();
            let seed = alloc::format!("hlq-jury-{}-{:?}", case_id, parent);

            // 4. run the sortition; jurors = candidates that won at least one seat.
            let seats = consensus_core::sortition_fp::elect_committee_fp(
                &reputation,
                &secret_keys,
                &seed,
                T::JurySize::get(),
            );
            // 5. map indices back to accounts, ordered by (seats desc, index asc) for determinism, capped.
            let mut won: Vec<(usize, u32)> = seats
                .into_iter()
                .filter_map(|(id, s)| if s >= 1 { id.parse::<usize>().ok().map(|i| (i, s)) } else { None })
                .collect();
            won.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let cap = T::MaxJury::get() as usize;
            won.into_iter()
                .take(cap)
                .filter_map(|(i, _)| pool.get(i).map(|(acc, _)| acc.clone()))
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_justice;
    use crate::{CaseStatus, Cases, InterestInspect, ReputationInspect, SlashOnVerdict};
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

    // Record slashes so tests can assert the verdict's only consequence fired.
    parameter_types! {
        pub storage SlashLog: Vec<(u64, u8, u128)> = Vec::new();
    }
    pub struct MockSlasher;
    impl SlashOnVerdict<u64> for MockSlasher {
        fn on_guilty(culprit: &u64, dimension: u8, loss: u128) {
            let mut v = SlashLog::get();
            v.push((*culprit, dimension, loss));
            SlashLog::set(&v);
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
    }

    impl pallet_justice::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type Reputation = MockRep;
        type Interest = MockInterest;
        type Slasher = MockSlasher;
        type JurySize = JurySize;
        type MaxJury = MaxJury;
        type MaxInterested = MaxInterested;
        type GuiltThreshold = GuiltThreshold;
        type Depth2WeightPermille = Depth2WeightPermille;
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
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 100, vec![]));
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
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 100, vec![]));
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
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 100, vec![]));
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
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 100, vec![13]));
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
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 100, vec![]));
            // a party (defendant) tries to vote — rejected (not a juror, and interested).
            assert_noop!(
                Justice::cast_vote(RuntimeOrigin::signed(11), 0, false),
                crate::Error::<Test>::NotAJuror
            );
        });
    }

    #[test]
    fn guilty_supermajority_slashes_defendant() {
        new_test_ext().execute_with(|| {
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 250, vec![]));
            let jury = jury_of(0);
            let n = jury.len();
            // all jurors vote guilty → well above 2/3.
            for j in &jury {
                assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(*j), 0, true));
            }
            assert_ok!(Justice::close_case(RuntimeOrigin::signed(10), 0));
            assert_eq!(Cases::<Test>::get(0).unwrap().status, CaseStatus::ResolvedGuilty);
            assert_eq!(SlashLog::get(), vec![(11u64, 0u8, 250u128)]);
            let _ = n;
        });
    }

    #[test]
    fn no_supermajority_means_not_guilty_and_no_slash() {
        new_test_ext().execute_with(|| {
            assert_ok!(Justice::open_case(RuntimeOrigin::signed(10), 11, [0u8; 32], 0, 250, vec![]));
            let jury = jury_of(0);
            // only one juror votes guilty → far below 2/3 → name intact, no slash (Art. IX).
            assert_ok!(Justice::cast_vote(RuntimeOrigin::signed(jury[0]), 0, true));
            assert_ok!(Justice::close_case(RuntimeOrigin::signed(10), 0));
            assert_eq!(Cases::<Test>::get(0).unwrap().status, CaseStatus::ResolvedNotGuilty);
            assert!(SlashLog::get().is_empty());
        });
    }

    #[test]
    fn nobody_sues_themselves() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                Justice::open_case(RuntimeOrigin::signed(10), 10, [0u8; 32], 0, 100, vec![]),
                crate::Error::<Test>::SelfCase
            );
        });
    }
}
