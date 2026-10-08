/* Harlequin — panel dashboard (post-login).  #610  "EL CAMERINO / THE DRESSING ROOM"
 *
 * Fills /panel with live data when the on-device wallet is unlocked: identity (handle + public key, zero
 * PII), balances (HLQ · La Bolsa / SOV · El Cofre), and standing (consensus reputation) shown APART from
 * balance — money ≠ power (Art. VI). Logged-out, the static preview stays exactly as a visitor sees it.
 *
 * OPSEC: pure UI. No seed/key/secret passes through here. The two live reads go through the panel's OWN
 * same-origin mediating backend (PANEL-ARCHITECTURE §80/§83/§99) — never a raw node RPC from the browser —
 * carrying the auth #611 cookie. Until the panel + backend deploy together the endpoints are empty, so the
 * page shows honest zeros / "—", never an invented figure. Loaded as an external file so a strict CSP
 * (`script-src 'self'`) never blocks it.
 */
(function () {
  "use strict";

  // Same-origin mediated backends (the operator's lane). Empty until deploy ⇒ honest zeros. At deploy:
  // "/api/balance" and "/api/standing".
  var BAL_ENDPOINT = "/api/balance";   // 7-oct-2026: wired (it was "" — every dressing room showed 0)
  // Live since 2026-09-23 (palanca nº 1): hlq-mediator reads the runtime's ReputationApi.
  var STANDING_ENDPOINT = "/api/standing";

  function W() { return window.HlqWallet; }
  function unlocked() { return !!(W() && W().isUnlocked()); }
  function $(id) { return document.getElementById(id); }

  // thousands separators (narrow no-break space) for an integer decimal string.
  function group(s) { return String(s).replace(/\B(?=(\d{3})+(?!\d))/g, " "); }
  // Base units (12 decimals) → «1 234,5» — same rule as the wallet: up to 4 decimals, dust shown as «< 0,0001».
  function coins(v) {
    if (!/^\d+$/.test(String(v))) return "—";
    var x = BigInt(v), U = 1000000000000n, w = x / U, f = x % U;
    var es = document.documentElement.classList.contains("lang-es"), d = es ? "," : ".";
    var dec = f.toString().padStart(12, "0").slice(0, 4).replace(/0+$/, "");
    if (w === 0n && !dec && f > 0n) return "< 0" + d + "0001";
    return group(w.toString()) + (dec ? d + dec : "");
  }

  // Set the number in a "<big>— <span class=u>UNIT</span>" card, keeping the unit span intact.
  function setBig(unit, text) {
    var bigs = document.querySelectorAll(".deck .card .big");
    for (var i = 0; i < bigs.length; i++) {
      var u = bigs[i].querySelector(".u");
      if (u && u.textContent === unit && bigs[i].firstChild) {
        bigs[i].firstChild.nodeValue = text + " ";
        return;
      }
    }
  }

  function getJSON(url) {
    var ctrl = new AbortController();
    var timer = setTimeout(function () { ctrl.abort(); }, 4000);
    return fetch(url, {
      method: "GET", headers: { "Accept": "application/json" },
      credentials: "same-origin", signal: ctrl.signal
    }).then(function (r) { clearTimeout(timer); if (!r.ok) throw new Error("http " + r.status); return r.json(); });
  }

  // --- identity: handle (what you share) + public key (technical id). Both PUBLIC; zero PII. ---
  function fillIdentity() {
    var addr = W().address();
    if ($("maskid") && addr) $("maskid").textContent = addr;        // handle hlq-…
    var pub = W().pubkeyHex && W().pubkeyHex();
    var el = $("maskpub");
    if (el && pub) { el.textContent = pub; el.hidden = false; }
  }

  // --- balances via the mediated backend; honest zeros when no endpoint, "—" on a backend error ---
  function fillBalances() {
    W().balances(BAL_ENDPOINT).then(function (b) {
      setBig("HLQ", coins(b.hlq));
      setBig("SOV", coins(b.sov));
    }).catch(function () {
      setBig("HLQ", "—"); setBig("SOV", "—");
    });
  }

  // --- standing (consensus reputation), apart from balance. Contract: docs/API-STANDING.md. The backend
  // may serve the aggregate alone ({standing, committee_eligible}) or, once the F4 runtime exposes the
  // per-suit vector (§1.2b), also {suits:{commerce,contribution,judgement,governance}} with values in
  // [0,1]. Per-suit bars fill via CSSOM width (allowed under strict CSP; no style attributes). No endpoint
  // ⇒ the honest "you start at zero" copy stays. ---
  function fillSuits(suits) {
    if (!suits) return;
    var lines = document.querySelectorAll(".suitline[data-suit]");
    for (var i = 0; i < lines.length; i++) {
      var v = suits[lines[i].getAttribute("data-suit")];
      if (v == null || isNaN(v)) continue;
      var pct = Math.max(0, Math.min(100, Math.round(Number(v) * 100)));
      var fill = lines[i].querySelector(".fill");
      if (fill) fill.style.width = pct + "%";
      lines[i].setAttribute("title", pct + "%");
    }
  }

  function fillStanding() {
    if (!STANDING_ENDPOINT) return;
    var pub = W().pubkeyHex && W().pubkeyHex();
    if (!pub) return;
    getJSON(STANDING_ENDPOINT + "?account=" + pub).then(function (s) {
      var v = (s && s.standing != null) ? s.standing : "0";
      var elig = !!(s && s.committee_eligible);
      var es = $("standing-val"), en = $("standing-val-en");
      if (es) es.textContent = " · " + v + (elig ? " · jurado: sí" : "");
      if (en) en.textContent = " · " + v + (elig ? " · committee: yes" : "");
      fillSuits(s && s.suits);
    }).catch(function () { /* leave the start-at-zero copy */ });
  }

  function render() {
    if (!unlocked()) return;                       // logged-out: the static preview is untouched
    var banner = document.querySelector(".preview-banner");
    if (banner) banner.hidden = true;              // the preview line is for visitors, not the owner
    fillIdentity();
    fillBalances();
    fillStanding();
  }

  // Nothing on /panel ever called unlock(): a citizen arriving here always saw the static preview. The
  // rito seals the mask WITHOUT a passphrase, so — exactly as the villa does since 2026-09-23 — a mask that
  // opens with the empty passphrase opens by itself. A mask WITH a lock fails closed here and the preview
  // stays (an unlock screen for those is a separate task).
  function openIfUnlocked() {
    if (!W() || unlocked()) return;
    W().hasKeystore().then(function (has) {
      if (has) return W().unlock("");                // fires hlqwallet-changed → render()
    }).catch(function () { /* locked mask: stays as preview */ });
  }

  function init() { if (W()) { render(); openIfUnlocked(); } }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
  window.addEventListener("hlqwallet-ready", function () { render(); openIfUnlocked(); }); // wasm ready
  window.addEventListener("hlqwallet-changed", render); // unlock / lock, same page
})();
