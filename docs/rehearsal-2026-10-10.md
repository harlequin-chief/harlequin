# Third rehearsal of the November 27th launch — 2026-10-10

The second rehearsal ([notes](rehearsal-2026-10-08.md)) put launch candidate 6 on the network. Two days later a simple
question — *does reputation actually work?* — was answered on the test network, and the answer was: not quite. Five
problems were found and fixed; the network was moved to a third rehearsal chain built from **launch candidate 7**. The
final chain still starts from a new ceremony on November 27th.

## What the test network showed

A society of bots lived on a separate six-node test network for a day: 306 forum posts, 90 deals, 46 payments, 78 sealed
pacts. Their reputation stayed at **zero**. By design reputation is never self-asserted — it comes from testimony of a
mask that already holds some, from vouches, and from measured node service — and the site offered no way to give the
first two. Looking closer:

1. **Finality votes were never credited as service.** The node signs its finality votes in the domain-separated v2 format;
   the participation pallet still verified the older format, so every vote was dropped. Measured on real
   justifications: v2 verifies 4/4, the old payload 0/4.
2. **A suit with no evidence anywhere was shared out evenly.** Once a suit's evidence had decayed to zero, a mask with 10
   units of technical evidence reached the standing of a founder. Now an empty suit trusts nobody.
3. **Reputable masks without a node could sit in the finality committee.** With the commit–reveal beacon still empty, the
   committee draw took every reputable account. Two node-less masks with reputation received seats they never used, and
   **finality on the test network stopped**. Now only masks with a node bound to them may sit; an empty list of bound
   nodes is an empty committee, never "everyone".
4. **…and could lead block slots.** With that fixed, finality held, but blocks slowed from 12 s to about 40 s: the slot
   leader draw also took node-less masks. Same rule now.
5. Hardening after an independent review: a genesis with reputable founders and no vote keys is refused (no committee
   could ever form), and the legacy vote window is closed (the launch chain is born on v2).

With candidate 7, the same script that froze the old binary (three node-less masks given reputation in all four suits)
ran for 28 minutes at a steady 12 s per block, finality never more than 8 blocks behind.

## Earning reputation from the site

The dressing room has a new card: claim the testimonies others gave you, testify for another mask or vouch for it in one
suit. A closed deal in the market offers «Testify for this deal». The page says beforehand when your word would weigh
nothing — a testimony weighs at most the evidence its giver holds in that suit, and only once the giver's overall
standing reaches the network's floor — and never sends a transaction that would add zero.

## Rehearsal chain 3

| Step | Result |
|---|---|
| Launch candidate 7 (tag `candidato-7-20261010`, runtime spec 5) | `.dist` sha256 `df4f2283…` (x86_64), `08bbdf42…` (aarch64), both path-clean; runtime `0c00bcb5…` |
| Source, published before the binaries | this repository and `harlequin-src-candidato-7.tar.gz` on `/dist` (`271969f0…`, deterministic) |
| Genesis, same ceremony script, same rehearsal accounts and session keys, fresh beacon | every gate passed; specification sha256 `d346ed19…`; genesis `0xe6f67e52a1449fb46c7006457cbdf36a066b8abf5b2e741fb903dbf78029f138` |
| The four validators moved to the new genesis (earlier rehearsal chains kept on disk) | first finality at block **#15** after about five and a half minutes |
| One validator did not start at first | a configuration file pointed at a network key that had not been copied; finality continued with three of four (exactly the threshold) until it was fixed |
| Installer and Docker image re-pinned | binary, specification, checkpoint = block zero |

## The newcomer's path, again

- Website walk: **13/13**; the villa map test: **14/14**.
- A stranger's clean machine runs the one-line installer: binary, specification, bind tool and heartbeat verified,
  checkpoint verified, node in step at about 150 MB. A mask born a minute earlier links that node through the villa:
  **21/21**.
- A Raspberry Pi 3 (1 GB) follows the chain with the aarch64 build of candidate 7 at about 170 MB.
