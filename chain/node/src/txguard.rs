//! Refuse rubbish at the door instead of letting the runtime trap on it (#30).
//!
//! WHY THIS EXISTS. A node hands the bytes of an incoming transaction to the runtime without looking
//! at them. If they are not a valid extrinsic, the runtime API entry decodes its arguments, fails,
//! and **panics** — stock Substrate behaviour, identical in Polkadot. The executor catches the trap
//! and the sender gets an error, so nothing breaks: measured on against the live binary
//! with 2,300 pieces of rubbish, the node did not die, ~1.1 ms of CPU each, and repeats are refused
//! by the pool itself.
//!
//! What it does cost is **noise**: 237 bytes of journal per piece of rubbish. The cheap attack is not
//! to knock a node down, it is to make it go quiet — bury the hours that matter under a flood. (The
//! first explanation of *how* it goes quiet, via journald's rate limit, was refuted by measurement on
//!: on our machines that limit scales with free disk and did not bite. The flood still
//! hurts, by a different mechanism. Diagnosis: `chain/DIAG-validate-transaction-panic-.md`.)
//!
//! The fix is to decode into the runtime's real extrinsic type **before** the pool ever calls the
//! runtime. Bytes that cannot be an extrinsic are rejected cleanly, with no trap and no panic line.
//! This is node-side only: it does not touch the runtime, does not change `spec_version` and needs no
//! ceremony — it rides in the next node binary like any other patch.
//!
//!
//! ⏸ STATUS (second pass): **SHAPE-BASED SCREEN, without touching `runtime/`.**
//! The first version wanted to decode into the REAL extrinsic, and that requires exporting a type the
//! runtime keeps private (`type Block`, `type TxExtension`). the maintainer approved "only `node/`, without touching the
//! chain", so that path stays **blocked behind its gate #45 (due 19-Aug)** and is NOT taken here.
//!
//! What can be done within what's approved: look at the **preamble**, whose rule lives in
//! `sp-runtime` (not in our runtime). An extrinsic starts with a version+type byte, and only
//! four combinations exist: bare v4/v5, signed v4, general v5. Any other first byte
//! **cannot be an extrinsic**, however it gets decoded.
//!
//! What it catches and what it doesn't — stated before anyone assumes otherwise:
//!   · CATCHES the rubbish measured on 13-Aug (`0x00`, `0x0400`, `0x08xxxx`, `0x10…`): its first byte is not
//!     a valid version. That kills 100% of what was tested in the lab.
//!   · DOES NOT CATCH a blob that starts with a valid byte and continues with garbage: that reaches the runtime and keeps
//!     tripping it up. That needs the full decode, i.e. #45.
//! It's a screen, not a fine sieve, and is documented as such.
//!
//! Mandatory validation before calling it good (the reviewer's criterion): measure **how many journal
//! lines a piece of rubbish writes BEFORE and AFTER**. "It looks cleaner" is not proof.
//! Both doors go through here: the RPC (`author_submitExtrinsic`) and the p2p transaction protocol
//! both reach the pool through this wrapper, which is the whole reason for wrapping the pool rather
//! than patching one entry point.

use std::{collections::HashMap, pin::Pin, sync::Arc};

// This crate goes through the `polkadot-sdk` umbrella (same as `service.rs`): the pool API is not a
// direct dependency, so it's imported from there or it doesn't compile.
//
// FIX (): `codec` does NOT hang off the `polkadot_sdk` umbrella — it was being requested as
// `polkadot_sdk::codec` and that path doesn't exist, so this file hasn't compiled since it was
// written. It comes from `sp_runtime`, which was already a direct dependency of this same module.
use polkadot_sdk::sc_transaction_pool_api::{
    error::Error as PoolError, ImportNotificationStream, PoolStatus, ReadyTransactions,
    TransactionFor, TransactionPool, TransactionSource, TransactionStatusStreamFor, TxHash,
    TxInvalidityReportMap,
};
use sp_runtime::codec::{self, Decode, Encode};
use sp_runtime::{traits::Block as BlockT, transaction_validity::InvalidTransaction};

/// Can this be an extrinsic, looking only at its preamble?
///
/// Rule from `sp-runtime` (`generic/unchecked_extrinsic.rs`): the first byte is version+type, with
/// `version = byte & 0b0011_1111` and `type = byte & 0b1100_0000`, and only these are valid: bare in v4/v5,
/// signed in v4, general in v5. We don't use any runtime type: that's why this fits within
/// "only `node/`".
fn looks_like_extrinsic(bytes: &[u8]) -> bool {
    // The pool handles opaque extrinsics: a `Vec<u8>` with a compact length prefix. If the blob
    // comes wrapped, it's looked at INSIDE; if not, it's looked at as-is. Checked against both forms
    // because the first version depended on that and it was unverified.
    match <Vec<u8> as Decode>::decode(&mut &bytes[..]) {
        Ok(unwrapped) => looks_like_preamble(&unwrapped),
        Err(_) => looks_like_preamble(bytes),
    }
}

fn looks_like_preamble(inner: &[u8]) -> bool {
    const VERSION_MASK: u8 = 0b0011_1111;
    const TYPE_MASK: u8 = 0b1100_0000;
    const BARE: u8 = 0b0000_0000;
    const SIGNED: u8 = 0b1000_0000;
    const GENERAL: u8 = 0b0100_0000;
    const LEGACY_VERSION: u8 = 4;
    const VERSION: u8 = 5;

    let Some(&first) = inner.first() else { return false };
    let version = first & VERSION_MASK;
    match (version, first & TYPE_MASK) {
        (LEGACY_VERSION..=VERSION, BARE) => true,
        (LEGACY_VERSION, SIGNED) => true,
        (VERSION, GENERAL) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::looks_like_preamble;

    #[test]
    fn the_rubbish_we_measured_is_refused() {
        // Exactly the blobs that on 13-Aug tripped up the runtime in the lab.
        for bad in [&[0x00u8][..], &[0x00, 0x00][..], &[0xde, 0xad][..], &[0xff][..], &[][..]] {
            assert!(!looks_like_preamble(bad), "should be rejected: {bad:?}");
        }
    }

    #[test]
    fn real_extrinsic_preambles_are_let_through() {
        for good in [0x04u8, 0x05, 0x84, 0x45] {
            assert!(looks_like_preamble(&[good, 0x00]), "should pass: {good:#04x}");
        }
    }
}

fn rejected<E: From<PoolError>>() -> E {
    // `Call` is the closest honest verdict for "these bytes are not an extrinsic at all": the sender
    // gets a clean invalid-transaction error instead of a wasm trap backtrace.
    E::from(PoolError::InvalidTransaction(InvalidTransaction::Call))
}

/// A transaction pool that refuses undecodable extrinsics before they reach the runtime.
pub struct GuardedPool<P>(pub Arc<P>);

impl<P> GuardedPool<P> {
    pub fn new(inner: Arc<P>) -> Arc<Self> {
        Arc::new(Self(inner))
    }
}

#[async_trait::async_trait]
impl<P> TransactionPool for GuardedPool<P>
where
    P: TransactionPool + Send + Sync + 'static,
    TransactionFor<P>: codec::Encode,
{
    type Block = P::Block;
    type Hash = P::Hash;
    type InPoolTransaction = P::InPoolTransaction;
    type Error = P::Error;

    async fn submit_at(
        &self,
        at: <Self::Block as BlockT>::Hash,
        source: TransactionSource,
        xts: Vec<TransactionFor<Self>>,
    ) -> Result<Vec<Result<TxHash<Self>, Self::Error>>, Self::Error> {
        // A batch arrives from a peer: keep the good ones, answer for the bad ones in their place, so
        // one rotten entry does not decide the fate of the whole batch.
        let verdicts: Vec<bool> = xts.iter().map(|xt| looks_like_extrinsic(&xt.encode())).collect();
        if verdicts.iter().all(|ok| *ok) {
            return self.0.submit_at(at, source, xts).await;
        }
        let good: Vec<_> = xts
            .into_iter()
            .zip(verdicts.iter())
            .filter_map(|(xt, ok)| ok.then_some(xt))
            .collect();
        let mut passed = if good.is_empty() {
            Vec::new().into_iter()
        } else {
            self.0.submit_at(at, source, good).await?.into_iter()
        };
        Ok(verdicts
            .into_iter()
            .map(|ok| {
                if ok {
                    passed.next().unwrap_or_else(|| Err(rejected()))
                } else {
                    Err(rejected())
                }
            })
            .collect())
    }

    async fn submit_one(
        &self,
        at: <Self::Block as BlockT>::Hash,
        source: TransactionSource,
        xt: TransactionFor<Self>,
    ) -> Result<TxHash<Self>, Self::Error> {
        if !looks_like_extrinsic(&xt.encode()) {
            return Err(rejected());
        }
        self.0.submit_one(at, source, xt).await
    }

    async fn submit_and_watch(
        &self,
        at: <Self::Block as BlockT>::Hash,
        source: TransactionSource,
        xt: TransactionFor<Self>,
    ) -> Result<Pin<Box<TransactionStatusStreamFor<Self>>>, Self::Error> {
        if !looks_like_extrinsic(&xt.encode()) {
            return Err(rejected());
        }
        self.0.submit_and_watch(at, source, xt).await
    }

    // ── Everything below is plain delegation: the guard only has an opinion about what comes IN. ──

    async fn ready_at(
        &self,
        at: <Self::Block as BlockT>::Hash,
    ) -> Box<dyn ReadyTransactions<Item = Arc<Self::InPoolTransaction>> + Send> {
        self.0.ready_at(at).await
    }

    fn ready(&self) -> Box<dyn ReadyTransactions<Item = Arc<Self::InPoolTransaction>> + Send> {
        self.0.ready()
    }

    async fn report_invalid(
        &self,
        at: Option<<Self::Block as BlockT>::Hash>,
        // FIX (): the trait requires `TxInvalidityReportMap` (an IndexMap), not a Vec of
        // pairs. This was written with the signature of a different API version, and that's why this
        // file didn't compile. This is pure delegation: it changes the type, not the behaviour.
        invalid_tx_errors: TxInvalidityReportMap<TxHash<Self>>,
    ) -> Vec<Arc<Self::InPoolTransaction>> {
        self.0.report_invalid(at, invalid_tx_errors).await
    }

    fn futures(&self) -> Vec<Self::InPoolTransaction> {
        self.0.futures()
    }

    fn status(&self) -> PoolStatus {
        self.0.status()
    }

    fn import_notification_stream(&self) -> ImportNotificationStream<TxHash<Self>> {
        self.0.import_notification_stream()
    }

    fn on_broadcasted(&self, propagations: HashMap<TxHash<Self>, Vec<String>>) {
        self.0.on_broadcasted(propagations)
    }

    fn hash_of(&self, xt: &TransactionFor<Self>) -> TxHash<Self> {
        self.0.hash_of(xt)
    }

    fn ready_transaction(&self, hash: &TxHash<Self>) -> Option<Arc<Self::InPoolTransaction>> {
        self.0.ready_transaction(hash)
    }

    async fn ready_at_with_timeout(
        &self,
        at: <Self::Block as BlockT>::Hash,
        timeout: std::time::Duration,
    ) -> Box<dyn ReadyTransactions<Item = Arc<Self::InPoolTransaction>> + Send> {
        self.0.ready_at_with_timeout(at, timeout).await
    }
}

/// `spawn_tasks` needs a *maintained* pool: it feeds it every imported block, and it also builds the
/// `author_*` RPC on top of it. Pure delegation — the guard has no opinion about block maintenance —
/// but without it the RPC door could not be guarded at all (, wiring the guard).
#[async_trait::async_trait]
impl<P> polkadot_sdk::sc_transaction_pool_api::MaintainedTransactionPool for GuardedPool<P>
where
    P: polkadot_sdk::sc_transaction_pool_api::MaintainedTransactionPool + Send + Sync + 'static,
    TransactionFor<P>: codec::Encode,
{
    async fn maintain(
        &self,
        event: polkadot_sdk::sc_transaction_pool_api::ChainEvent<Self::Block>,
    ) {
        self.0.maintain(event).await
    }
}
