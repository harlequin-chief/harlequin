# Harlequin node — runtime image.
# The binary is fetched and sha256-verified at build time (same security model as install-harlequin-node.sh):
# a tampered binary fails the check and the build aborts. No secrets are baked in; the node runs unprivileged.
#
# Base pinned by DIGEST (not just a tag) so the image is reproducible and can't be swapped under us.
# Pinned 2026-07-21 (post-launch) to the current debian:trixie-slim manifest-list digest; re-pin on
# each release with:  curl the registry manifest and read docker-content-digest for trixie-slim.
FROM debian:trixie-slim@sha256:020c0d20b9880058cbe785a9db107156c3c75c2ac944a6aa7ab59f2add76a7bd

ARG BIN_URL=https://harlequinproject.org/dist/harlequin-node
# MUST agree with BIN_SHA_x86_64 in install-harlequin-node.sh. It did not, for six weeks: this line
# still pinned the launch binary (fa79dda9, 18-jul) while the network moved to fb50ba6a and then to
# c3172c55. A pin that is only a comment away from the installer's is not a pin, it is a second
# truth — and this one was two runtime generations behind the chain it claims to join.
# Re-pinned 2026-08-27 to the build running on all four nodes since 2026-08-03.
ARG BIN_SHA256=ad04a91715e8cac987978ed56d62b648812edda081aff65565c43e524194d5b4

# The chain spec is verified here too, not just mounted on trust. `docker build` alone must be enough
# to prove BOTH halves of what the node runs; asking the operator to check the spec by hand elsewhere
# is asking them to skip it.
ARG CHAINSPEC_URL=https://harlequinproject.org/dist/mainnet-raw.json
ARG CHAINSPEC_SHA256=ba1b25f7179d24c89aabd0a5f924d06f15365e1040cff3be2811a771e42086a6

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl; \
    rm -rf /var/lib/apt/lists/*; \
    curl -fsS --proto '=https' --tlsv1.2 "$BIN_URL" -o /usr/local/bin/harlequin-node; \
    echo "${BIN_SHA256}  /usr/local/bin/harlequin-node" | sha256sum -c -; \
    chmod 0755 /usr/local/bin/harlequin-node; \
    install -d /etc/harlequin; \
    curl -fsS --proto '=https' --tlsv1.2 "$CHAINSPEC_URL" -o /etc/harlequin/mainnet-raw.json; \
    echo "${CHAINSPEC_SHA256}  /etc/harlequin/mainnet-raw.json" | sha256sum -c -; \
    apt-get purge -y curl; apt-get autoremove -y; \
    useradd --system --home-dir /data --shell /usr/sbin/nologin harlequin; \
    install -d -o harlequin -g harlequin /data

USER harlequin
WORKDIR /data
EXPOSE 30333
# The spec now ships INSIDE the image, verified above; a compose file may still mount its own over it.
ENTRYPOINT ["harlequin-node"]
# These flags are the SAME SET the systemd unit uses in install-harlequin-node.sh. They were not, and
# the difference was invisible because nobody ever put the boot paths side by side: this one was missing
# the consensus engine, the network backend, --no-mdns, the bootnodes, --blocks-pruning and the txpool
# fix. Four doors into the same network, written separately, is four chances to be wrong once each.
# --state-pruning archive: REQUIRED so the epoch-start state the finality committee is read from is never
# pruned (default 256 → committee read returns empty past ~256 blocks → finality stalls, the #270 bug).
# --pool-type single-state: fixes the "Essential task txpool-background failed" crash, which bites
# precisely while catching up at full speed — what every newcomer does on their first day.
CMD ["--base-path", "/data", "--chain", "/etc/harlequin/mainnet-raw.json", \
     "--node-key-file", "/data/node-key", "--name", "hlq-docker-node", "--port", "30333", \
     "--consensus", "woven-trust-12000", \
     "--network-backend", "libp2p", \
     "--no-mdns", \
     "--bootnodes", \
       "/ip4/148.116.86.24/tcp/30333/p2p/12D3KooWBiZXWDXuXKzKw8f6Wpmo3Mx81oAgFu2VG1fsZihs3BHC", \
       "/ip4/151.145.42.146/tcp/30333/p2p/12D3KooWLinJp4ZZnrcGsqXprk7snpdC64KnnTPenN2CP356z37X", \
     "--state-pruning", "archive", \
     "--blocks-pruning", "archive", \
     "--pool-type", "single-state"]
