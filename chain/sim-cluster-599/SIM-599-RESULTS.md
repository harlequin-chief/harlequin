# SIM #599 — cluster-guard trajectories (activation-decision input)

**Date:** · **Author:** the author · **Code:** `chain/sim-cluster-599/` (workspace member) ·
**Raw output:** `OUTPUT-.txt` · **Pipeline:** the REAL `reputation-core` code path
(EigenTrust fully-fixed-fp with community+concentration damping → per-suit `communities()` label
propagation → `max_cluster_share_fp` → the wired arming rule + `entrenchment_halt_step`), never
re-derived math. Seeded from the LIVE premeasure of (mainnet `0xa1ab`: 4 founders,
equal shares ≈25.03%, zero vouch edges, unarmed). Deterministic schedules, no randomness.

**Model notes.** Evidence accrues ADDITIVELY (attest→claim pays per epoch of honest service);
founders keep serving at +25/epoch, an honest external earns +60/epoch. One dimension simulated —
the pallet takes the worst of four and every scenario here is suit-symmetric. Mainnet epoch =
600 blocks × 12 s = **2 h** (12 epochs/day).

## Findings

**F1 — A CONNECTED society still splits into communities; arming is reachable organically (S5).**
The core fear — label propagation fusing a sponsor-tree society into ONE community, cluster share
≈100% forever, the leg never arming — is REFUTED with the real algorithm: a fully connected web
(every newcomer vouched by a founder, then by earlier externals; joins every 4 epochs) arms at
**epoch 169 ≈ 14 days**. Sub-communities emerge naturally as the web grows.

**F2 — Arming horizon scales with onboarding rate (S1).** Unvouched-loner worst case: one join
every 4 epochs → arms at e121 (~10 days); every 12 epochs (≈1/day) → e311 (~26 days); newcomers
vouch-pairing with each other accelerates both (e107 / e273). Slower onboarding delays arming but
never blocks it; while unarmed, single-entity keeps covering (each founder ~25%, margin to 33%).

**F3 — The vouch premium is real and load-bearing.** With identical evidence, vouched members hold
a large EigenTrust reputation premium over unvouched loners (the founder ring held ~60% of rep on
~29% of raw evidence). Consequences: (a) evidence ratios are NOT rep ratios — never reason about
the guard from raw evidence; (b) newcomers only weigh once vouched INTO the web — onboarding-with-
vouches is what actually dilutes the founders.

**F4 — Post-arm, founder re-concentration is economically absurd (S2).** In a ~46-member society,
the 4 founders re-crossing 1/3 requires sustaining ≈**30×** their normal service rate — and even
then the counter starts only after ~377 post-arm epochs and the halt lands 7 epochs later (~e383,
freeze correct and self-heal owed by #918). At 10× they never re-cross. Entrenchment cost grows
with society size: the design works.

**F5 — Sybil rings are caught only by the cluster leg, on a horizon inverse to their greed (S3).**
8 masks each serving at 5× the honest rate (every mask individually FAR under 1/3 — single-entity
is blind): counter starts ~e333 post-arm, HALT ~e339 (~28 days of sustained attack). At 10× rate,
or 16 masks at 5×: HALT in ~103 epochs (~8.6 days). The faster they grab, the faster the freeze.

**F6 — Declared residual, quantified (S4).** One entity split into 3 UNVOUCHED masks (no edges
between them) holds ~32.5% per mask ≈97% total, `real_cluster=false` — invisible to the cluster
leg AND under the single-entity bar. Known, declared (#754 v2): needs behavioural clustering
(co-spend/co-vote patterns), not graph clustering. Not worsened by #599; pre-existing blind spot.

## Reading for the activation decision (post-#918)
1. Ship #599 observe-only now (already APT): while unarmed, single-entity covers and the live set
   sits at 25.03% with margin.
2. Expect organic arming within **~2–4 weeks of real onboarding** (F1/F2) — the ClusterGuard
   events will show the countdown publicly (Art X).
3. Before FULL activation, #918 must be fixed: F4/F5 halts are correct detections whose self-heal
   is currently broken node-side.
4. #754 v2 (behavioural anti-split) is the remaining real gap (F6) — schedule after #918.
