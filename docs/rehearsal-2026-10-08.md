# Second rehearsal of the November 27th launch — 2026-10-08

The first rehearsal (2026-09-28, [notes](rehearsal-2026-09-28.md)) ran the launch recipe with launch candidate 4 and then 5.
Since then the launch branch gained pacts, two-sided market deals, the hosting reward and a light-node profile. A
rehearsal that does not run the binary that will actually launch proves less than it seems, so the network was moved
again, to a second rehearsal chain built from **launch candidate 6**. The final chain still starts from a new ceremony on
November 27th.

## What was done

| Step | Result |
|---|---|
| Genesis built with the same ceremony script, the same rehearsal accounts and session keys, a fresh beacon and the two public bootnodes | every gate passed (no development keys, public bootnodes only, 4 vote keys wired, manifesto sealed in block zero) |
| Genesis hash | `0xecdaa7d302af9f0e8a071c780460bf55faa164f8a9fb30df64b36aeab9c13aef` |
| Chain specification (`mainnet-raw.json`) | sha256 `be8082320c5f54900c284616846e5ab15f3fd8adabf66b93b436dc53cc4fa1bc` |
| Node binary: launch candidate 6 (tag `candidato-6-20261007`) | `.dist` sha256 `4b5d4715…` (x86_64), `ede44f0f…` (aarch64); the runtime both embed is the same wasm, `afbf0637…` |
| Reproducibility | two independent cold builds in the canonical tree gave the x86_64 binary and the runtime bit for bit |
| The four validators moved to the new genesis (the first rehearsal chain is kept intact on disk) | first finality at block **#15**, **4 of 4** signing |
| Source of the running binary | this repository's `chain/` and `harlequin-src-candidato-6.tar.gz` on `/dist`, published before the binaries |
| Installer and Docker image re-pinned (binary, spec, weak-subjectivity checkpoint = block zero) | the installer kept since October 1st (light profile on small disks) is now the public one |

## The newcomer's path, tested as a newcomer

- **Website walk**, rite → mask → villa login → dressing room → reputation → leave: **13/13** in Spanish and in English.
- **A stranger's machine**: a clean 1 GB container runs the one-line installer from the website. Binary, specification,
  bind tool and checkpoint are verified; the node chooses the light profile and follows finality at about 213 MB.
- **Linking a node to a mask**: a mask born a minute earlier links its node through the villa and later replaces it, with
  the bind tool downloaded from `/dist`: **21/21**.
- **A physical ARM board**: a Raspberry Pi 3 with 1 GB of memory follows the chain with the light profile at about 200 MB.
  With the installer's default caches its first sync of the larger first rehearsal chain needed about 380 MB, which is
  too much for a 1 GB board that does anything else; small boards need smaller caches.

## What now works on the public site

- **Closing a deal in the market**: the buyer proposes it from the offer, the seller confirms it, and the chain records it
  with both signatures. No money moves inside a deal; paying is a separate transfer.
- **The Notary seals a pact**: once every party has signed the pact package, the villa can record its sealed key on chain.
  The date is public; the text, the salt and who signed with whom stay with the parties.
- **Wallet activity** is read from the chain's own events, and the site asks the runtime which calls exist instead of
  hard-coding them, so features a chain does not have are not offered.

## A society of bots

To see everything working together, ten bots lived on a separate six-node test network with real finality and the launch
runtime at test cadence. They used the same wallet code and transaction builder as the website and talked only to the
mediator, like a browser. Every action counted only when it could be read back from the chain: 798 verified actions
(names, forum posts and replies, offers, deals, payments, gifts, withdrawals, pacts, sealed agreements) and 11 that did not
confirm (1.4%): two with a known cause, already fixed, and nine not yet classified.

They also showed something about the economy: on a network whose validators are all founders nobody is ever paid, because
founders collect zero by protocol. On the real chain the first coin is minted when the first era closes, about six months
after genesis (whitepaper v4, §9 and §12). Until then everything on the feeless lane works and nothing can be paid in HLQ.
Whether the first era should be shorter is an open decision.

## Found on the way

- The runtime's own lockfile lacked one of the new pallets, so a `--locked` build of the runtime would have refused to
  start. Fixed before the tag.
- A villa button that should hide could never hide: a more specific style rule overrode the hiding class. The browser test
  had checked the class, not what is on screen; it now checks the computed style, and fails on the old style sheet.
