//! `HlqCheckNonce` — drop-in replacement for `frame_system::CheckNonce` with ONE deviation:
//! the permissionless cold-start lane (SPEC-RELAUNCH §2, gate B2).
//!
//! Upstream `CheckNonce` rejects any signed extrinsic from an account with zero `providers` and zero
//! `sufficients` (`InvalidTransaction::Payment`, "nonce storage not paid for") BEFORE the payment
//! extension ever runs. On a chain where coin can only be EARNED, that makes a fresh mask's first
//! extrinsic unpayable by construction: it can never attest/commit to earn its first reputation —
//! the exact chicken-and-egg §2 exists to kill.
//!
//! Deviation (marked `HLQ:` below; everything else is upstream `frame-system 47.0.0
//! src/extensions/check_nonce.rs` with `T` fixed to our `Runtime`):
//!   * `validate`: an account with no providers/sufficients is allowed through ONLY when the call is
//!     `HlqTxPayment::feeless_eligible` AND the declared nonce is 0. Everything else keeps the
//!     upstream rejection. The feeless BUDGET (per-mask allowance + global 15% block ceiling) is
//!     enforced right after by `ChargeTransactionPayment` → `can_withdraw_fee`, which runs in the
//!     SAME pool-validation pipeline — so the pool never admits more than the budget allows.
//!   * `prepare`: the same newborn case mints the account's existence with
//!     `frame_system::inc_sufficients` before the nonce is written, so the nonce record persists.
//!     If a later extension (e.g. payment) rejects the extrinsic, the whole application rolls back —
//!     existence is only committed together with a successfully included extrinsic.
//!
//! Anti-farm unchanged (R1): a farm of fresh masks buys `allowance(age=0)=1` slot per mask per budget
//! epoch, capped globally by the 15% feeless block lane; paying traffic outbids it in the pool.

extern crate alloc;

use alloc::{vec, vec::Vec};

use crate::{interface, HlqTxPayment, Runtime, RuntimeCall};
use codec::{Decode, DecodeWithMemTracking, Encode};
use frame::deps::frame_support::{dispatch::DispatchInfo, pallet_prelude::TransactionSource};
use frame::deps::frame_support::weights::Weight;
use frame::deps::sp_runtime::{
    traits::{
        AsSystemOriginSigner, DispatchInfoOf, One, PostDispatchInfoOf, TransactionExtension,
        ValidateResult, Zero,
    },
    transaction_validity::{
        InvalidTransaction, TransactionLongevity, TransactionValidityError, ValidTransaction,
    },
    DispatchResult,
};
use scale_info::TypeInfo;

/// Nonce check and increment to give replay protection for transactions (upstream contract),
/// plus the HLQ newborn-mask lane documented above.
#[derive(Encode, Decode, DecodeWithMemTracking, Clone, Eq, PartialEq, TypeInfo)]
pub struct HlqCheckNonce(#[codec(compact)] pub interface::Nonce);

/// For a valid transaction the provides and requires information related to the nonce.
pub struct ValidNonceInfo {
    pub provides: Vec<Vec<u8>>,
    pub requires: Vec<Vec<u8>>,
}

impl HlqCheckNonce {
    /// HLQ: the ONE gate that differs from upstream — may this (account, call, nonce) create its own
    /// account record? Only a first-ever (nonce 0) extrinsic riding the feeless lane.
    fn newborn_allowed(call: &RuntimeCall, nonce: interface::Nonce) -> bool {
        HlqTxPayment::feeless_eligible(call) && nonce.is_zero()
    }

    /// Upstream `validate_nonce_for_account` + the newborn lane.
    fn validate_nonce_for_account(
        who: &interface::AccountId,
        call: &RuntimeCall,
        nonce: interface::Nonce,
    ) -> Result<ValidNonceInfo, TransactionValidityError> {
        let account = frame_system::Account::<Runtime>::get(who);
        if account.providers.is_zero() && account.sufficients.is_zero() {
            // Upstream: nonce storage not paid for → reject.
            // HLQ: unless this is a newborn mask's first feeless-eligible extrinsic (budget is
            // checked by the payment extension later in this same validation pipeline).
            if !Self::newborn_allowed(call, nonce) {
                return Err(InvalidTransaction::Payment.into());
            }
        }
        if nonce < account.nonce {
            return Err(InvalidTransaction::Stale.into());
        }

        let provides = vec![Encode::encode(&(who.clone(), nonce))];
        let requires = if account.nonce < nonce {
            vec![Encode::encode(&(who.clone(), nonce.saturating_sub(One::one())))]
        } else {
            vec![]
        };

        Ok(ValidNonceInfo { provides, requires })
    }

    /// Upstream `prepare_nonce_for_account` + the newborn lane (account creation before the nonce
    /// write, atomically inside the extrinsic's storage transaction). Returns whether this extrinsic
    /// CREATED the account, so `post_dispatch_details` can undo the creation if the dispatch fails.
    fn prepare_nonce_for_account(
        who: &interface::AccountId,
        call: &RuntimeCall,
        mut nonce: interface::Nonce,
    ) -> Result<bool, TransactionValidityError> {
        let account = frame_system::Account::<Runtime>::get(who);
        let mut created = false;
        if account.providers.is_zero() && account.sufficients.is_zero() {
            // HLQ: mirror of the validate-time gate at block-build time.
            if !Self::newborn_allowed(call, nonce) {
                return Err(InvalidTransaction::Payment.into());
            }
            // HLQ: the mask starts to EXIST by transacting (self-sufficient reference); rolled back
            // with the whole extrinsic if any later extension rejects it.
            let _ = frame_system::Pallet::<Runtime>::inc_sufficients(who);
            created = true;
        }
        if nonce > account.nonce {
            return Err(InvalidTransaction::Future.into());
        }
        nonce = nonce
            .checked_add(interface::Nonce::one())
            .unwrap_or(interface::Nonce::zero());
        frame_system::Account::<Runtime>::mutate(who, |account| account.nonce = nonce);
        Ok(created)
    }
}

impl core::fmt::Debug for HlqCheckNonce {
    #[cfg(feature = "std")]
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "HlqCheckNonce({})", self.0)
    }

    #[cfg(not(feature = "std"))]
    fn fmt(&self, _: &mut core::fmt::Formatter) -> core::fmt::Result {
        Ok(())
    }
}

/// Operation to perform from `validate` to `prepare` (upstream shape).
pub enum Val {
    /// Account to check for (the declared nonce travels in `self.0`).
    CheckNonce(interface::AccountId),
    /// Weight to refund (unsigned/inherent path — untouched from upstream).
    Refund(Weight),
}

/// Operation to perform from `prepare` to `post_dispatch_details` (upstream shape, plus the HLQ
/// newborn flag).
pub enum Pre {
    NonceChecked,
    /// HLQ: THIS extrinsic created `who`'s account via the newborn lane. If the dispatch fails, the
    /// creation is undone in `post_dispatch_details` — existence only outlives its first extrinsic
    /// when that extrinsic SUCCEEDS (review H1: no empty permanent accounts, no reaper).
    NonceCheckedNewborn(interface::AccountId),
    Refund(Weight),
}

impl TransactionExtension<RuntimeCall> for HlqCheckNonce {
    const IDENTIFIER: &'static str = "CheckNonce";
    type Implicit = ();
    type Val = Val;
    type Pre = Pre;

    fn weight(&self, _: &RuntimeCall) -> Weight {
        <<Runtime as frame_system::Config>::ExtensionsWeightInfo as frame_system::ExtensionsWeightInfo>::check_nonce()
    }

    fn validate(
        &self,
        origin: <Runtime as frame_system::Config>::RuntimeOrigin,
        call: &RuntimeCall,
        _info: &DispatchInfoOf<RuntimeCall>,
        _len: usize,
        _self_implicit: Self::Implicit,
        _inherited_implication: &impl Encode,
        _source: TransactionSource,
    ) -> ValidateResult<Self::Val, RuntimeCall> {
        // Unsigned/inherent path: untouched from upstream — no account, nothing to check.
        let Some(who) = origin.as_system_origin_signer() else {
            return Ok((Default::default(), Val::Refund(self.weight(call)), origin));
        };
        let ValidNonceInfo { provides, requires } =
            Self::validate_nonce_for_account(who, call, self.0)?;

        let validity = ValidTransaction {
            priority: 0,
            requires,
            provides,
            longevity: TransactionLongevity::max_value(),
            propagate: true,
        };

        Ok((validity, Val::CheckNonce(who.clone()), origin))
    }

    fn prepare(
        self,
        val: Self::Val,
        _origin: &<Runtime as frame_system::Config>::RuntimeOrigin,
        call: &RuntimeCall,
        _info: &DispatchInfoOf<RuntimeCall>,
        _len: usize,
    ) -> Result<Self::Pre, TransactionValidityError> {
        let (who, nonce) = match val {
            Val::CheckNonce(who) => (who, self.0),
            Val::Refund(weight) => return Ok(Pre::Refund(weight)),
        };
        Self::prepare_nonce_for_account(&who, call, nonce).map(|created| {
            if created {
                Pre::NonceCheckedNewborn(who)
            } else {
                Pre::NonceChecked
            }
        })
    }

    fn post_dispatch_details(
        pre: Self::Pre,
        _info: &DispatchInfo,
        _post_info: &PostDispatchInfoOf<RuntimeCall>,
        _len: usize,
        result: &DispatchResult,
    ) -> Result<Weight, TransactionValidityError> {
        match pre {
            Pre::NonceChecked => Ok(Weight::zero()),
            // HLQ: a newborn account only outlives its first extrinsic if that extrinsic SUCCEEDS.
            // On failure, dropping our sufficient ref takes it to providers=0/sufficients=0 →
            // reaped in this same block (`on_killed_account`), nothing dangles. Retry is bounded:
            // the feeless budget spent on the failed attempt persists (extension-side write).
            Pre::NonceCheckedNewborn(who) => {
                if result.is_err() {
                    let _ = frame_system::Pallet::<Runtime>::dec_sufficients(&who);
                }
                Ok(Weight::zero())
            }
            Pre::Refund(weight) => Ok(weight),
        }
    }
}

// The B2 hole lived between the pallets and the tx pipeline (unit tests passed, the pool rejected),
// so these tests run against the REAL `Runtime`, not a mock.
#[cfg(test)]
mod tests {
    use super::*;

    fn newborn() -> interface::AccountId {
        interface::AccountId::from([7u8; 32])
    }

    fn feeless_call() -> RuntimeCall {
        RuntimeCall::BeaconPallet(pallet_beacon::Call::commit {
            commitment: [1u8; 32],
        })
    }

    fn paying_call() -> RuntimeCall {
        // Any call OUTSIDE the feeless allowlist — one that exists in BOTH build flavors
        // (sudo does not: it is compiled out of mainnet).
        RuntimeCall::Tokens(pallet_tokens::Call::transfer {
            coin: pallet_tokens::Coin::Hlq,
            to: interface::AccountId::from([9u8; 32]),
            amount: 1,
        })
    }

    fn ext() -> frame::deps::sp_io::TestExternalities {
        frame::deps::sp_io::TestExternalities::default()
    }

    #[test]
    fn newborn_feeless_first_extrinsic_passes_and_creates_account() {
        ext().execute_with(|| {
            let who = newborn();
            // upstream would reject here with InvalidTransaction::Payment (check_nonce.rs:75)
            HlqCheckNonce::validate_nonce_for_account(&who, &feeless_call(), 0)
                .expect("newborn + feeless + nonce 0 must validate");
            HlqCheckNonce::prepare_nonce_for_account(&who, &feeless_call(), 0)
                .expect("newborn + feeless + nonce 0 must prepare");
            let account = frame_system::Account::<Runtime>::get(&who);
            assert_eq!(account.sufficients, 1, "mask must now exist (self-sufficient)");
            assert_eq!(account.nonce, 1, "nonce must persist after creation");
        });
    }

    #[test]
    fn newborn_non_feeless_call_still_rejected_as_upstream() {
        ext().execute_with(|| {
            let got = HlqCheckNonce::validate_nonce_for_account(&newborn(), &paying_call(), 0);
            assert_eq!(
                got.err(),
                Some(InvalidTransaction::Payment.into()),
                "non-feeless call from unfunded account keeps the upstream rejection"
            );
        });
    }

    #[test]
    fn newborn_feeless_with_nonzero_nonce_rejected() {
        ext().execute_with(|| {
            let got = HlqCheckNonce::validate_nonce_for_account(&newborn(), &feeless_call(), 1);
            assert_eq!(
                got.err(),
                Some(InvalidTransaction::Payment.into()),
                "only a FIRST (nonce 0) extrinsic may create the account"
            );
        });
    }

    #[test]
    fn existing_account_behaves_exactly_like_upstream() {
        ext().execute_with(|| {
            let who = newborn();
            let _ = frame_system::Pallet::<Runtime>::inc_providers(&who);
            // stale nonce → Stale (not Payment): the newborn lane must not touch this path
            frame_system::Account::<Runtime>::mutate(&who, |a| a.nonce = 5);
            let got = HlqCheckNonce::validate_nonce_for_account(&who, &paying_call(), 4);
            assert_eq!(got.err(), Some(InvalidTransaction::Stale.into()));
            // current nonce validates + prepares, and does NOT add sufficients
            HlqCheckNonce::validate_nonce_for_account(&who, &paying_call(), 5).expect("valid");
            HlqCheckNonce::prepare_nonce_for_account(&who, &paying_call(), 5).expect("prepared");
            let account = frame_system::Account::<Runtime>::get(&who);
            assert_eq!(account.nonce, 6);
            assert_eq!(account.sufficients, 0, "existing account must not gain sufficients");
        });
    }

    #[test]
    fn future_nonce_rejected_in_prepare_as_upstream() {
        ext().execute_with(|| {
            let who = newborn();
            let _ = frame_system::Pallet::<Runtime>::inc_providers(&who);
            let got = HlqCheckNonce::prepare_nonce_for_account(&who, &paying_call(), 3);
            assert_eq!(got.err(), Some(InvalidTransaction::Future.into()));
        });
    }

    // ---- review H1: newborn existence dies with a failed first dispatch ----

    fn post_dispatch(pre: Pre, result: &DispatchResult) {
        HlqCheckNonce::post_dispatch_details(
            pre,
            &DispatchInfo::default(),
            &Default::default(),
            0,
            result,
        )
        .expect("post_dispatch_details never errors");
    }

    #[test]
    fn newborn_failed_dispatch_reaps_the_account() {
        ext().execute_with(|| {
            let who = newborn();
            let created =
                HlqCheckNonce::prepare_nonce_for_account(&who, &feeless_call(), 0).unwrap();
            assert!(created, "prepare must report the creation");
            assert!(frame_system::Account::<Runtime>::contains_key(&who));
            post_dispatch(
                Pre::NonceCheckedNewborn(who.clone()),
                &Err(frame::deps::sp_runtime::DispatchError::BadOrigin),
            );
            assert!(
                !frame_system::Account::<Runtime>::contains_key(&who),
                "failed first dispatch → account reaped in the same block, nothing dangles"
            );
        });
    }

    #[test]
    fn newborn_successful_dispatch_keeps_the_account() {
        ext().execute_with(|| {
            let who = newborn();
            let created =
                HlqCheckNonce::prepare_nonce_for_account(&who, &feeless_call(), 0).unwrap();
            assert!(created);
            post_dispatch(Pre::NonceCheckedNewborn(who.clone()), &Ok(()));
            let account = frame_system::Account::<Runtime>::get(&who);
            assert_eq!(account.sufficients, 1, "successful first dispatch → the mask stays");
            assert_eq!(account.nonce, 1);
        });
    }

    #[test]
    fn failed_dispatch_never_decrements_a_preexisting_account() {
        ext().execute_with(|| {
            let who = newborn();
            let _ = frame_system::Pallet::<Runtime>::inc_sufficients(&who);
            let created =
                HlqCheckNonce::prepare_nonce_for_account(&who, &feeless_call(), 0).unwrap();
            assert!(!created, "existing account → not a newborn");
            post_dispatch(
                Pre::NonceChecked,
                &Err(frame::deps::sp_runtime::DispatchError::BadOrigin),
            );
            let account = frame_system::Account::<Runtime>::get(&who);
            assert_eq!(
                account.sufficients, 1,
                "a failed dispatch must never touch refs this tx did not create"
            );
        });
    }
}
