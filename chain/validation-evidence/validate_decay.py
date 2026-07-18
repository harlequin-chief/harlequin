#!/usr/bin/env python3
"""
validate_decay.py — IRON validation of the decay mechanism (#33) on the devnet
binary (ρ=0.90 placeholder). Confirms that under advance_epoch the Evidence of an
account that does NOT re-inject falls ~0.90x/epoch (enters at full weight, evaporates),
while an account that DOES re-inject every epoch holds. Validates the leaky
integrator (Σ raw·ρ^(t-s)).
"""
import sys
from substrateinterface import SubstrateInterface, Keypair

URL = "ws://127.0.0.1:9944"
SUIT = "Commerce"
AMOUNT = 1000               # u128; control re-injection (= genesis seed per suit)
RHO = 0.90
N_EPOCHS = 8

SS58 = 42
s = SubstrateInterface(url=URL, ss58_format=SS58)
alice = Keypair.create_from_uri("//Alice", ss58_format=SS58)
bob = Keypair.create_from_uri("//Bob", ss58_format=SS58)
charlie = Keypair.create_from_uri("//Charlie", ss58_format=SS58)
print(f"ss58_format={SS58}  Alice(sudo)={alice.ss58_address}")
print(f"Bob(decays)={bob.ss58_address}\nCharlie(control, re-injects)={charlie.ss58_address}")


def sudo_call(module, function, params):
    inner = s.compose_call(call_module=module, call_function=function, call_params=params)
    call = s.compose_call(call_module="Sudo", call_function="sudo", call_params={"call": inner})
    ext = s.create_signed_extrinsic(call=call, keypair=alice)
    r = s.submit_extrinsic(ext, wait_for_inclusion=True)
    ok = r.is_success
    if not ok:
        print(f"  ⚠️ {module}.{function} FAILED: {r.error_message}")
    return ok


def inject(target_ss58):
    return sudo_call("Reputation", "submit_evidence",
                     {"who": target_ss58, "suit": SUIT, "amount": AMOUNT})


def advance():
    return sudo_call("Reputation", "advance_epoch", {})


def read_ev(ss58):
    v = s.query("Reputation", "Evidence", [ss58, SUIT])
    return int(v.value) if v.value is not None else 0


def read_rep(ss58):
    v = s.query("Reputation", "ReputationSnapshot", [ss58, SUIT])
    return int(v.value) if v.value is not None else 0


def epoch():
    return int(s.query("Reputation", "Epoch", []).value)


print("\n== evidence SEEDED at genesis (6 founders, 1000/suit) ==")
print(f"epoch={epoch()}  Bob.ev={read_ev(bob.ss58_address)} (decays, does NOT re-inject)"
      f"  Charlie.ev={read_ev(charlie.ss58_address)} (control, re-injects)")
# Bob: pure decay of the genesis seed. Nothing is injected.

print("\n== advancing epochs (Bob does NOT re-inject, Charlie DOES) ==")
print(f"{'ep':>3} {'Bob.ev':>12} {'ratio':>7} {'Bob.rep':>12} {'Charlie.ev':>12} {'Char.rep':>12}")
prev_bob = read_ev(bob.ss58_address)
e0_bob = prev_bob
e0 = epoch()
rows = []
for i in range(N_EPOCHS):
    # the control re-injects BEFORE advancing
    inject(charlie.ss58_address)
    advance()
    be = read_ev(bob.ss58_address); br = read_rep(bob.ss58_address)
    ce = read_ev(charlie.ss58_address); cr = read_rep(charlie.ss58_address)
    ratio = be / prev_bob if prev_bob else 0.0
    print(f"{epoch():>3} {be:>12} {ratio:>7.4f} {br:>12} {ce:>12} {cr:>12}")
    rows.append((be, ratio, ce))
    prev_bob = be

print("\n== VERDICT ==")
import statistics
ratios = [r for (_, r, _) in rows]  # Bob epoch-to-epoch ratio (pure decay)
avg = statistics.mean(ratios) if ratios else 0.0
print(f"mean Bob.ev epoch-to-epoch ratio = {avg:.4f}  (expected ≈ {RHO})")
bob_genesis = e0_bob
bob_final = rows[-1][0]; char_final = rows[-1][2]
mech_ok = abs(avg - RHO) < 0.02 and bob_final < bob_genesis * 0.5 and char_final >= bob_genesis
print(f"Bob.ev genesis={bob_genesis} → final={bob_final} (decays) · Charlie.ev final={char_final} (holds/rises)")
print("RESULT:", "✅ DECAY MECHANISM OK" if mech_ok else "❌ review")
sys.exit(0 if mech_ok else 1)
