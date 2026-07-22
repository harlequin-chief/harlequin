# Harlequin node — runtime image.
# The binary is fetched and sha256-verified at build time (same security model as install-harlequin-node.sh):
# a tampered binary fails the check and the build aborts. No secrets are baked in; the node runs unprivileged.
#
# Base pinned by DIGEST (not just a tag) so the image is reproducible and can't be swapped under us.
# Pinned (post-launch) to the current debian:trixie-slim manifest-list digest; re-pin on
# each release with:  curl the registry manifest and read docker-content-digest for trixie-slim.
FROM debian:trixie-slim@sha256:020c0d20b9880058cbe785a9db107156c3c75c2ac944a6aa7ab59f2add76a7bd

ARG BIN_URL=https://harlequinproject.org/dist/harlequin-node
# Matches the LAUNCH binary served at /dist/harlequin-node (fa79dda9, verified against the VPS
#). Re-pin together with the installer's BIN_SHA_x86_64 on every release — they must agree.
ARG BIN_SHA256=fa79dda97e0f335eed274ec89c7a80c51ef7ff60eb979f11a76df7336e6666a6

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl; \
    rm -rf /var/lib/apt/lists/*; \
    curl -fsS --proto '=https' --tlsv1.2 "$BIN_URL" -o /usr/local/bin/harlequin-node; \
    echo "${BIN_SHA256}  /usr/local/bin/harlequin-node" | sha256sum -c -; \
    chmod 0755 /usr/local/bin/harlequin-node; \
    apt-get purge -y curl; apt-get autoremove -y; \
    useradd --system --home-dir /data --shell /usr/sbin/nologin harlequin; \
    install -d -o harlequin -g harlequin /data

USER harlequin
WORKDIR /data
EXPOSE 30333
# --chain spec is mounted read-only by compose; data + node-key live in the /data volume.
ENTRYPOINT ["harlequin-node"]
# --state-pruning archive: REQUIRED so the epoch-start state the finality committee is read from is never
# pruned (default 256 → committee read returns empty past ~256 blocks → finality stalls, the #270 bug).
CMD ["--base-path", "/data", "--chain", "/etc/harlequin/mainnet-raw.json", \
     "--node-key-file", "/data/node-key", "--name", "hlq-docker-node", "--port", "30333", \
     "--state-pruning", "archive"]
