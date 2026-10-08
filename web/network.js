/* Harlequin network viewer.
 *
 * Reads the EpochReport telemetry that the nodes themselves serve (SPEC §5c) — there is no central
 * server. Point TELEMETRY_URL at a public node's telemetry endpoint once the chain is live; until then
 * the viewer shows an honest offline state. The EpochReport shape is the one emitted by `protocol-core`:
 *   { epoch, active_nodes, committee_size, total_reputation, reputation_by_suit:{...}, gini, finalized }
 */
(function () {
  "use strict";

  // Same-origin aggregate served from the node distribution (node-dist/hlq-stats.py → network.json),
  // OPSEC-clean by construction: counts/heights/aggregate state, never an IP, host or location. Absent
  // until the public chain is live ⇒ the fetch fails and the page shows an honest offline state.
  var TELEMETRY_URL = "/network.json";
  var POLL_MS = 10000;

  var $ = function (id) { return document.getElementById(id); };

  // Active language (mirrors app.js, which toggles `html.lang-es`). The injected status/timestamp text
  // must follow it too — otherwise the Spanish site shows "offline"/"updated" in English.
  function lang() { return document.documentElement.classList.contains("lang-es") ? "es" : "en"; }
  function t(es, en) { return lang() === "es" ? es : en; }

  var lastLive = false;
  var lastUpdated = null; // Date of the last successful render, so the timestamp re-localises on toggle.
  // Block numbers whose detail row the visitor has opened. Kept across the 10s polls so a re-render
  // does not collapse a block the visitor is reading.
  var expanded = Object.create(null);


  function setStatus(live) {
    lastLive = live;
    var dot = $("statusDot");
    if (dot) dot.className = "dot" + (live ? " live" : "");
    var el = $("statusText");
    if (el) el.textContent = live ? t("en vivo", "live") : t("desconectado", "offline");
  }

  function showOffline(on) {
    ["offline-es", "offline-en"].forEach(function (id) {
      var el = $(id);
      if (el) el.style.display = on ? "" : "none";
    });
    // Hide the live-data dashboard while offline so the page reads as a clean, honest
    // "not live yet" message — not a broken-looking skeleton of "—" placeholders and empty bars.
    // `.livechain` carries its own statgrid + empty block list, so it must be hidden explicitly
    // (querySelector(".statgrid") only matches the first/header grid, never the live-chain one).
    [".statgrid", ".livechain"].forEach(function (sel) {
      var el = document.querySelector(sel);
      if (el) el.style.display = on ? "none" : "";
    });
  }

  // Consumes the flat, OPSEC-clean contract that the nodes serve via hlq-stats.py:
  //   { chain, bestBlock, finalizedBlock, peers, syncing, recentBlocks[{n,hash,fin,x}], updated }
  // Aggregate only — heights, hashes and a peer count. NEVER an IP, peer-id, node identity or location,
  // and never the trust graph. The richer per-epoch telemetry (committee, reputation, gini) is not served
  // here by design: it would either need identities we refuse to publish or runtime APIs not yet wired.
  function render(r) {
    showOffline(false);
    setStatus(true);

    var lag = (r.bestBlock != null && r.finalizedBlock != null)
      ? Math.max(0, r.bestBlock - r.finalizedBlock) : null;

    if ($("chainname")) $("chainname").textContent = r.chain || "";
    if ($("m-best")) $("m-best").textContent = r.bestBlock != null ? "#" + r.bestBlock : "—";
    if ($("m-final2")) $("m-final2").textContent = r.finalizedBlock != null ? "#" + r.finalizedBlock : "—";
    if ($("m-lag")) {
      $("m-lag").textContent = lag == null ? "—"
        : lag + " " + (lag === 1 ? t("bloque", "block") : t("bloques", "blocks"));
    }


    var blockList = $("blocks");
    if (blockList && Array.isArray(r.recentBlocks)) {
      // Built with safe DOM methods (textContent) + validated fields — no innerHTML, no injection.
      // Each block opens to reveal its full hash and finality. We never serve WHO did WHAT inside a
      // block — that would de-anonymise members. The explorer shows the block's identity, not its contents.
      blockList.textContent = "";
      var prevN = null;
      r.recentBlocks.forEach(function (b) {
        var n = parseInt(b.n, 10);
        if (!isFinite(n)) return;
        // The contract serves two clusters (live tip + finalised frontier); when the finality lag is
        // large they are non-contiguous, so draw a "…" gap rather than implying missing blocks.
        if (prevN !== null && prevN - n > 1) {
          var gap = document.createElement("li");
          gap.className = "bgap";
          var gt = document.createElement("span");
          gt.textContent = "⋯ " + (prevN - n - 1) + " " + t("bloques", "blocks");
          gap.appendChild(gt);
          blockList.appendChild(gap);
        }
        prevN = n;
        var full = (typeof b.hash === "string" && /^0x[0-9a-fA-F]+$/.test(b.hash)) ? b.hash : "";
        var short = full ? full.slice(0, 10) + "…" + full.slice(-6) : "";
        var li = document.createElement("li");
        li.className = b.fin ? "fin" : "pend";

        // Clickable header row (a real <button> for keyboard + screen-reader support).
        var head = document.createElement("button");
        head.type = "button"; head.className = "brow";
        var open = expanded[n] === true;
        head.setAttribute("aria-expanded", open ? "true" : "false");
        var sn = document.createElement("span"); sn.className = "bn"; sn.textContent = "#" + n;
        var sh = document.createElement("span"); sh.className = "bh"; sh.textContent = short;
        // Operations count in the row itself: it was buried in the detail panel, where nobody saw that
        // the chain is doing work. A ledger of hashes with no activity reads like a screensaver.
        var sx = document.createElement("span"); sx.className = "bx";
        sx.textContent = (typeof b.x === "number" && b.x >= 0) ? String(b.x) : "";
        // Words, not glyphs: "✓ / ·" needed a legend to be read at all. "Sealed" says what finality
        // MEANS to a visitor — this block can no longer be undone.
        var sf = document.createElement("span"); sf.className = "bf";
        sf.textContent = b.fin ? t("sellado", "sealed") : t("en curso", "pending");
        head.appendChild(sn); head.appendChild(sh); head.appendChild(sx); head.appendChild(sf);

        // Detail panel: full hash + finality. Only public chain identity, never block contents.
        var det = document.createElement("div"); det.className = "bdetail"; det.hidden = !open;
        function field(labelEs, labelEn, val, mono) {
          var row = document.createElement("div"); row.className = "bdfield";
          var lab = document.createElement("span"); lab.className = "bdlabel";
          lab.textContent = t(labelEs, labelEn);
          var v = document.createElement("span"); v.className = "bdval" + (mono ? " mono" : "");
          v.textContent = val;
          row.appendChild(lab); row.appendChild(v); return row;
        }
        det.appendChild(field("Bloque", "Block", "#" + n, false));
        if (full) det.appendChild(field("Huella", "Fingerprint", full, true));
        det.appendChild(field("Estado", "Status",
          b.fin ? t("sellado", "sealed") : t("en curso", "pending"), false));
        // Extrinsic COUNT only (aggregate; the feed never carries bodies/senders/args).
        if (typeof b.x === "number" && b.x >= 0) {
          det.appendChild(field("Operaciones", "Operations", String(b.x), false));
        }

        head.addEventListener("click", function () {
          var now = !(expanded[n] === true);
          expanded[n] = now;
          det.hidden = !now;
          head.setAttribute("aria-expanded", now ? "true" : "false");
        });

        li.appendChild(head); li.appendChild(det);
        blockList.appendChild(li);
      });
    }

    // Freshness WITHOUT publishing our clock: the feed serves `ageSeconds` (how old the reading is).
    // We anchor it to the browser's own clock at fetch time, so the label keeps counting between polls.
    // Fallback to the old absolute field while a node still serves the previous format.
    if (typeof r.ageSeconds === "number") {
      lastUpdated = new Date(Date.now() - r.ageSeconds * 1000);
    } else {
      lastUpdated = (r.updated_at || r.updated) ? new Date(r.updated_at || r.updated) : new Date();
    }
    renderUpdated();
  }

  // The "updated …" timestamp, in the active language. Kept separate so a language toggle re-localises it.
  function renderUpdated() {
    var el = $("updated");
    if (!el || !lastUpdated) return;
    // OPSEC (2026-08-01, a finding by the maintainer): this used to publish the server clock's ABSOLUTE
    // timestamp ("updated 2026-07-30 18:01:36 UTC"). That gives away two things for free: the machine's
    // exact time and, by reading the page every few minutes, the rhythm it is operated at — when someone
    // is at the keyboard and when not. The visitor only cares about one thing: whether the data is fresh.
    // It is stated in relative terms, which answers that without publishing our clock.
    var secs = Math.max(0, Math.round((Date.now() - lastUpdated.getTime()) / 1000));
    var txt;
    if (secs < 60) txt = t("hace un momento", "just now");
    else if (secs < 3600) {
      var m = Math.round(secs / 60);
      txt = t("hace " + m + " min", m + " min ago");
    } else {
      var h = Math.round(secs / 3600);
      txt = t("hace " + h + " h", h + " h ago");
    }
    el.textContent = t("actualizado ", "updated ") + txt;
  }

  function poll() {
    if (!TELEMETRY_URL) { setStatus(false); showOffline(true); return; }
    fetch(TELEMETRY_URL, { cache: "no-store" })
      .then(function (resp) { if (!resp.ok) throw new Error("http " + resp.status); return resp.json(); })
      .then(render)
      .catch(function () { setStatus(false); showOffline(true); });
  }

  // Re-localise the injected text when the language toggle is used (app.js handles the static spans).
  document.querySelectorAll(".lang button").forEach(function (b) {
    b.addEventListener("click", function () { setStatus(lastLive); renderUpdated(); });
  });

  poll();
  if (TELEMETRY_URL) setInterval(poll, POLL_MS);
})();
