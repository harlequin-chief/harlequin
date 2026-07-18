# Run a Harlequin node

Everything you need to run a node on the live Harlequin network. A node syncs and serves the chain;
it needs **no account** — your mask (your identity in the society) is created separately, by you, and
never touches a node.

## Quick install (Linux, x86_64 or aarch64)

```sh
curl -fsSL https://harlequinproject.org/install-harlequin-node.sh | sh
```

The script is the security boundary: it pins the sha256 of the node binary and of the sealed launch
chainspec, verifies both before running anything, and pins a finalized **weak-subjectivity checkpoint**
so a fresh node cannot be fed a forged fork. Read it before piping it to a shell — and verify the pins
against at least two independent sources (this script over HTTPS, the [network page](https://harlequinproject.org/network),
and an operator you already trust). If the sources disagree, stop.

Run it behind a VPN: a node announces its IP to peers.

## Docker

```sh
docker compose up -d
```

`Dockerfile` + `docker-compose.yml` build and run the same pinned binary. The network `node-key` is
generated on first run into the persistent volume (`0600`) — it is a network identity, not an account
key, and is never an account or a secret you must guard like a phrase.

## Publishing telemetry (optional)

`hlq-stats.py` reads a *local* node's JSON-RPC and writes a small, aggregate `network.json` (best block,
finalized block, peer count, recent block hashes + extrinsic counts) for a public status page. It
publishes **only aggregates** — never peer IPs, peer-ids, node names or the trust graph.

## Genesis

Verify the chain you join is the real one at <https://harlequinproject.org/genesis> — the genesis hash,
the Bitcoin-anchored beacon (recomputable in your browser), and the sealed manifesto.
