/* genesis.js — the /genesis page's two live behaviours, both dependency-free and network-free.
 *
 * 1) Recompute the genesis beacon in the visitor's own browser: SHA256(commit_bytes ‖ btc_hash_bytes)
 *    via WebCrypto, compared to the sealed value. Nothing is trusted from us — the two inputs are
 *    public and the visitor watches the hash fall out.
 * 2) Copy-to-clipboard on each hash capsule.
 *
 * Strict CSP: external script with 'self', no inline, events bound in JS. OPSEC: only public constants
 * (a committed phrase-hash and a Bitcoin block hash — both meant to be public) and a pure hash.
 */
(function () {
  "use strict";

  var COMMIT = "500b4ef5d4618f6ea186f7eca6b2b506088ad7380751333ad24a9d56f6ae41ba";
  var BTC_HASH = "00000000000000000000243dc3f55c70fb5d032b93a099ee64f49fe5baca331d";
  var EXPECTED = "40600756ebde265c0f01308299cf1ee6ce4054f1fdb82d3d5b4ac8254482fd17";

  function hexToBytes(hex) {
    var out = new Uint8Array(hex.length / 2);
    for (var i = 0; i < out.length; i++) out[i] = parseInt(hex.substr(i * 2, 2), 16);
    return out;
  }
  function bytesToHex(buf) {
    var b = new Uint8Array(buf), s = "";
    for (var i = 0; i < b.length; i++) s += b[i].toString(16).padStart(2, "0");
    return s;
  }
  function isES() { return document.documentElement.getAttribute("lang") === "es"; }

  // ── 1. recompute the beacon ────────────────────────────────────────────
  function verify(btn, result, hashEl) {
    if (!window.crypto || !window.crypto.subtle) {
      hashEl.textContent = isES()
        ? "Tu navegador no permite el cálculo aquí — usa la fórmula con cualquier herramienta SHA-256."
        : "Your browser can't compute here — use the formula with any SHA-256 tool.";
      result.hidden = false;
      return;
    }
    var commit = hexToBytes(COMMIT), btc = hexToBytes(BTC_HASH);
    var input = new Uint8Array(commit.length + btc.length);
    input.set(commit, 0);
    input.set(btc, commit.length);
    btn.disabled = true;
    window.crypto.subtle.digest("SHA-256", input).then(function (digest) {
      var got = bytesToHex(digest);
      hashEl.textContent = got;
      result.hidden = false;
      // match → keep the ✓ card as designed; mismatch would flip the mark (never happens with sealed inputs)
      if (got !== EXPECTED) {
        var check = result.querySelector(".gx-check");
        if (check) check.textContent = "✗";
      }
      btn.disabled = false;
    }).catch(function () {
      hashEl.textContent = isES() ? "No se pudo calcular en este navegador." : "Couldn't compute in this browser.";
      result.hidden = false;
      btn.disabled = false;
    });
  }

  // ── 2. copy capsules ───────────────────────────────────────────────────
  function wireCopy(btn) {
    btn.addEventListener("click", function () {
      var cap = btn.closest(".gx-cap");
      var val = cap && cap.getAttribute("data-copy");
      if (!val) return;
      var done = function () {
        btn.classList.add("copied");
        setTimeout(function () { btn.classList.remove("copied"); }, 1400);
      };
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(val).then(done).catch(function () {});
      } else {
        var ta = document.createElement("textarea");
        ta.value = val; ta.style.position = "fixed"; ta.style.opacity = "0";
        document.body.appendChild(ta); ta.select();
        try { document.execCommand("copy"); done(); } catch (e) {}
        document.body.removeChild(ta);
      }
    });
  }

  function init() {
    var btn = document.getElementById("gx-verify-btn");
    var result = document.getElementById("gx-verify-result");
    var hashEl = document.getElementById("gx-verify-hash");
    if (btn && result && hashEl) {
      btn.addEventListener("click", function () { verify(btn, result, hashEl); });
    }
    var copies = document.querySelectorAll(".gx-copy");
    for (var i = 0; i < copies.length; i++) wireCopy(copies[i]);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
