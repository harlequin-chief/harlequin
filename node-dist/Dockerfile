# Harlequin node — runtime image.
# The binary is fetched and sha256-verified at build time (same security model as install-harlequin-node.sh):
# a tampered binary fails the check and the build aborts. No secrets are baked in; the node runs unprivileged.
#
# Base pinned by DIGEST (not just a tag) so the image is reproducible and can't be swapped under us.
# PIN-BEFORE-PUBLISH: replace the digest below with the current debian:trixie-slim digest at release time.
FROM debian:trixie-slim@sha256:PIN-BEFORE-PUBLISH

ARG BIN_URL=https://harlequinproject.org/dist/harlequin-node
ARG BIN_SHA256=231bc5ca86e618d19b6e744d6b1b24ce7f37272d25ad0dd55b3d88ee537673fc

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
