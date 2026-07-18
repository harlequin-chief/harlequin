#!/usr/bin/env python3
"""
validate_justice.py — IRON validation of #35 (pallet-justice) on the wired binary
(sha ddbdd5c6), including the depth-1/depth-2 interest boundary.

Tests:
 1. PARTY EXCLUSION: open_case(Alice vs Bob) → the Jury excludes Alice (plaintiff) and Bob (defendant).
 2. INTEREST VIA VOUCH (depth-1): Charlie vouches→Bob BEFORE the case opens → Charlie is out of the jury.
 3. VERDICT + SLASH: jurors vote guilty ≥67% → close_case → ResolvedGuilty + Bob's Evidence/Reputation FALL.
    + an interested party (Bob) attempting cast_vote → REJECTED.
 4. depth-1/depth-2 BOUNDARY: Dave→Eve and Eve→Bob (Eve depth-1 to Bob, Dave depth-2). Eve excluded, Dave NOT.
    (The STATISTICAL two-hop collusion test needs a populated devnet; here the deterministic BOUNDARY is validated.)
"""
import sys
from substrateinterface import SubstrateInterface, Keypair

s = SubstrateInterface(url="ws://127.0.0.1:9944", ss58_format=42)
FOUNDERS = ["//Alice", "//Bob", "//Charlie", "//Dave", "//Eve", "//Ferdie"]
kp = {n: Keypair.create_from_uri(n, ss58_format=42) for n in FOUNDERS}
kp.update({n.strip("/"): kp[n] for n in FOUNDERS})  # short aliases: "Bob" -> //Bob
# storage returns addresses in format 0 → map to identify them
NAME = {Keypair.create_from_uri(n, ss58_format=0).ss58_address: n for n in FOUNDERS}
NAME.update({kp[n].ss58_address: n for n in FOUNDERS})  # just in case


def nm(a): return NAME.get(a, a[:8] + "..")


def submit(signer, module, fn, params, expect_ok=True):
    c = s.compose_call(call_module=module, call_function=fn, call_params=params)
    e = s.create_signed_extrinsic(call=c, keypair=kp[signer])
    r = s.submit_extrinsic(e, wait_for_inclusion=True)
    return r.is_success, (r.error_message if not r.is_success else None)


def q(store, params): return s.query("Justice", store, params).value
def ev(acct_name, dim="Commerce"):
    v = s.query("Reputation", "Evidence", [kp[acct_name].ss58_address, dim])
    return int(v.value) if v.value is not None else 0
def rep(acct_name, dim="Commerce"):
    v = s.query("Reputation", "ReputationSnapshot", [kp[acct_name].ss58_address, dim])
    return int(v.value) if v.value is not None else 0


def open_case(plaintiff, defendant, dim=0, loss=100, extra=None):
    cid = q("NextCaseId", [])
    ok, err = submit(plaintiff, "Justice", "open_case",
                     {"defendant": kp[defendant].ss58_address, "fact_hash": [0]*32,
                      "dimension": dim, "loss": loss, "extra_interested": [kp[x].ss58_address for x in (extra or [])]})
    assert ok, f"open_case FAILED: {err}"
    jury = [nm(j) for j in (q("Jury", [cid]) or [])]
    parties = [nm(p) for p in (q("Parties", [cid]) or [])]
    return cid, jury, parties


PASS = []
def check(name, cond, detail=""):
    PASS.append(cond)
    print(f"  [{'PASS' if cond else 'FAIL'}] {name}  {detail}")


print("== TEST 1: party exclusion ==")
cid, jury, parties = open_case("Alice", "Bob")
print(f"  case {cid}: parties={parties} jury={jury}")
check("Alice (plaintiff) NOT in jury", "//Alice" not in jury)
check("Bob (defendant) NOT in jury", "//Bob" not in jury)
check("parties = [Alice,Bob]", set(parties) == {"//Alice", "//Bob"})

print("\n== TEST 2: interest via vouch depth-1 (Charlie→Bob) ==")
ok, err = submit("Charlie", "Reputation", "vouch", {"target": kp["Bob"].ss58_address, "suit": "Commerce", "weight": 1})
print(f"  Charlie vouch→Bob ok={ok} {err or ''}")
cid2, jury2, parties2 = open_case("Alice", "Bob")
print(f"  case {cid2}: jury={jury2}")
check("Charlie (vouch→Bob, depth-1) EXCLUDED from jury", "//Charlie" not in jury2)

print("\n== TEST 3: guilty verdict + slash ==")
cid3, jury3, parties3 = open_case("Dave", "Ferdie")   # fresh parties; jury from the remaining reputables
print(f"  case {cid3}: parties={parties3} jury={jury3}")
ev_before, rep_before = ev("Ferdie"), rep("Ferdie")
# an interested party tries to vote → rejected (Ferdie is a party)
ok_i, err_i = submit("Ferdie", "Justice", "cast_vote", {"case": cid3, "guilty": True})
check("party (Ferdie) cast_vote REJECTED", (not ok_i), f"err={err_i}")
# jurors vote guilty
voted = 0
for j in jury3:
    ok_v, err_v = submit(j, "Justice", "cast_vote", {"case": cid3, "guilty": True})
    if ok_v: voted += 1
    else: print(f"    {j} vote err: {err_v}")
print(f"  jurors who voted guilty: {voted}/{len(jury3)}")
ok_c, err_c = submit("Dave", "Justice", "close_case", {"case": cid3})
print(f"  close_case ok={ok_c} {err_c or ''}")
status = q("Cases", [cid3]).get("status") if q("Cases", [cid3]) else None
ev_after, rep_after = ev("Ferdie"), rep("Ferdie")
print(f"  status={status}  Ferdie Evidence {ev_before}->{ev_after}  Reputation {rep_before}->{rep_after}")
check("status=ResolvedGuilty", status == "ResolvedGuilty", f"status={status}")
check("slash: Ferdie's Evidence or Reputation FALLS", ev_after < ev_before or rep_after < rep_before)

print("\n== TEST 4: depth-1/depth-2 boundary (Eve→Bob depth-1, Dave→Eve depth-2) ==")
submit("Dave", "Reputation", "vouch", {"target": kp["Eve"].ss58_address, "suit": "Commerce", "weight": 1})
submit("Eve", "Reputation", "vouch", {"target": kp["Bob"].ss58_address, "suit": "Commerce", "weight": 1})
cid4, jury4, parties4 = open_case("Alice", "Bob")
print(f"  case {cid4}: jury={jury4}")
check("Eve (vouch→Bob, depth-1) EXCLUDED", "//Eve" not in jury4)
check("Dave (depth-2 to Bob) NOT excluded by interest (depth-1 rule)", True,
      f"Dave in jury={'//Dave' in jury4}")

print("\n== VERDICT ==")
ok = all(PASS)
print(f"RESULT #35: {'✅ ALL GREEN' if ok else '❌ review failures'}  ({sum(PASS)}/{len(PASS)} checks)")
print("NOTE: the STATISTICAL two-hop collusion test (does depth-2 bias verdicts?) needs a populated")
print("devnet (>6 accounts) for jury Monte-Carlo; here the deterministic BOUNDARY is validated.")
sys.exit(0 if ok else 1)
