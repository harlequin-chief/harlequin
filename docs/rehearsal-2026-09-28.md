# Dress rehearsal of the November 27th launch — 2026-09-28

On November 27th, 2026 Harlequin starts its final chain from a new genesis. To find the problems before that
day instead of on it, we ran the whole launch recipe **on the real network, two months early**: new genesis,
the launch candidate binary, the installer, the downloads, the website and the monitoring — all of it switched
to a rehearsal chain. That chain will be replaced by the final one on launch day.

## What was done

| Step | Result |
|---|---|
| Genesis ceremony (the same script the launch will use) with fresh rehearsal keys, the manifesto sealed in block zero, two public bootnodes | every gate passed (no development keys, no private or loopback bootnodes, 4 vote keys wired) |
| Genesis hash | `0x95cff149be56a08cc97547f7347e93d66a4bcebfa1ef2ab07a73205f164c5df9` |
| Chain specification (`mainnet-raw.json`) | sha256 `0b83ded5619e3fda0a872fd9acce78aac0d38b67f881e81630aa58ff0a3fa69a` |
| Node binary: launch candidate 4 | `.dist` sha256 `3a4a34e9…` (x86_64), `6abbbda2…` (aarch64) |
| The four validators switched to the new genesis, one by one | first block after seconds, **first finality at block #15 six minutes in**, 4 of 4 signing |
| Source of the running binary | published (this repository's `chain/` and a tarball on `/dist`), **rebuilt from it on a clean machine: same sha** |
| Installer re-pinned (binary, spec, weak-subjectivity anchor = block zero) | verified end to end |

## The newcomer's path, tested as a newcomer

- **Website walk**: rite → mask → villa login → dressing room → reputation read from the chain → leave:
  **13/13** in Spanish and **13/13** in English, on the live site.
- **A stranger's machine**: a clean Debian with **1 GB of RAM** runs the one-line installer from the website.
  It downloads and verifies the binary, the specification and the anchor, syncs in seconds, and a mask born
  with **zero coins** takes a name, links its own node, speaks and makes an offer on its first day: **4/4**.
- **Linking a node to a mask** in one command (`harlequin-link-to-mask`) plus the villa: **21/21**, including
  replacing the node later. The villa now confirms the link by reading the chain instead of trusting that a
  transaction was sent.

## What the rehearsal caught

**A gossip storm in finality.** A node that follows finality by importing proofs (every newcomer, every
follower, and a validator when another one finalises first) kept its gossip watermark at the height it
started with. It stored every vote and proof it ever heard, and the gossip layer re-sends everything stored to
every peer every 750 ms. Measured on the 1 GB newcomer over ten minutes: 203,178 proofs and 25,004 votes in,
135,370 votes out, while only 51 block requests were answered and sync stalled. Three and a half hours after
genesis a bootnode was already sending about 237 KiB/s for a four-node chain; since nothing stored is ever
dropped, the traffic can only grow with the age of the chain (inferred from the mechanism, measured once). It had stayed hidden on the previous chain because every upgrade restarted the nodes.

The fix (watermarks follow local finality; what is far ahead of the blocks a node has is processed but neither
stored nor relayed) is launch candidate 5 (`.dist` `6a38ef47…` x86_64, `b4059793…` aarch64). It only counts once it
runs on **every** node: a fixed newcomer still drowns if its peers keep flooding it.

Tested before touching the live network, same lab run for both binaries (four validators, a newcomer joining after
block #120):

| | candidate 4 | candidate 5 |
|---|---|---|
| newcomer's finality after 15 minutes | block 0 | at the head, like the network |
| votes relayed by the newcomer | 154,781 | 3,382 |
| newcomer's first-day path | 4/4 | 4/4 |

Then rolled out to the four live validators one at a time, each only after the network was signing 4 of 4. The
bootnode that was sending 237 KiB/s now sends about 51 KiB/s.

## Why publish this

Because the whole point of Harlequin is that you do not have to trust us. A rehearsal that only reported
successes would be a brochure.
