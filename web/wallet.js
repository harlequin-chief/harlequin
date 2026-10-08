/* Harlequin — wallet view ("La Bolsa" HLQ / "El Cofre" SOV).  #616 / M3 #664/#666
 *
 * Render layer over `HlqWallet` (wallet-core.js — the on-device WASM seam). Keys live on the device; the
 * project custodies nothing and quotes no fiat value. Balances are READ-only and (in production) verified
 * on-device — a compromised server can DISPLAY false balances but can never move funds (no custody).
 *
 * States: locked (no wallet) → "—", actions off, "create/unlock" affordance; unlocked → canonical handle,
 * balances poll, Send/Receive live (7-oct-2026: /api/balance + wallet-send.js).
 */
(function () {
  "use strict";

  // Same-origin balance backend endpoint (PANEL-ARCHITECTURE §83/§99: a mediating backend does the
  // node state_call; the browser NEVER talks raw RPC). Set to "/api/balance" once the backend is live;
  // empty until then ⇒ honest zeros. (#616, the operator's backend lane.)
  var BAL_ENDPOINT = "/api/balance";   // 7-oct-2026: wired (it was "" — every wallet showed 0)
  var POLL_MS = 12000;

  var $ = function (id) { return document.getElementById(id); };
  function lang() { return document.documentElement.classList.contains("lang-es") ? "es" : "en"; }
  function t(es, en) { return lang() === "es" ? es : en; }

  var W = null;
  var state = null;           // { hlq, sov }
  var lastUpdated = null;

  // Group an integer-digit string with the locale thousands separator (no Number → no u128 precision loss).
  function groupInt(digits, sep) { return digits.replace(/\B(?=(\d{3})+(?!\d))/g, sep); }
  function amount(v) {
    if (v == null) return "—";
    var sep = lang() === "es" ? "." : ",";
    // Real u128 balances arrive as base-unit strings: 12 decimals (1 HLQ = 10^12). Show the whole part grouped and up
    // to 4 decimals, trimmed; a non-zero dust amount below that shows as «< 0,0001» instead of a lying 0.
    if (typeof v === "bigint" || (typeof v === "string" && /^\d+$/.test(v))) {
      var x = BigInt(v), U = 1000000000000n, whole = x / U, frac = x % U;
      var dec = (frac.toString().padStart(12, "0").slice(0, 4)).replace(/0+$/, "");
      var dsep = lang() === "es" ? "," : ".";
      if (whole === 0n && !dec && frac > 0n) return "< 0" + dsep + "0001";
      return groupInt(whole.toString(), sep) + (dec ? dsep + dec : "");
    }
    var n = Number(v);
    if (!isFinite(n)) return "—";
    return n.toLocaleString(lang() === "es" ? "es-ES" : "en-US");
  }
  function unlocked() { return !!(W && W.isUnlocked && W.isUnlocked()); }

  function setBtnLabel(es, en) {
    var btn = $("wallet-unlock");
    if (!btn) return;
    var e = btn.querySelector(".es"), n = btn.querySelector(".en");
    if (e) e.textContent = es;
    if (n) n.textContent = en;
  }

  function renderAddress() {
    var el = $("wallet-addr");
    if (!el) return;
    var a = "";
    if (unlocked()) { try { a = W.address() || ""; } catch (x) { a = ""; } }
    el.textContent = a || "—";
  }

  function setActionsEnabled(on) {
    document.querySelectorAll(".p-actions button").forEach(function (b) { b.disabled = !on; });
  }

  function render() {
    renderAddress();
    if ($("hlq-bal")) $("hlq-bal").textContent = state ? amount(state.hlq) : "—";
    if ($("sov-bal")) $("sov-bal").textContent = state ? amount(state.sov) : "—";
    setActionsEnabled(unlocked() && !!BAL_ENDPOINT);
    var btn = $("wallet-unlock");
    if (btn) btn.style.display = unlocked() ? "none" : "";
  }

  function renderUpdated() {
    var el = $("updated");
    if (!el || !lastUpdated) return;
    el.textContent = t("actualizado ", "updated ") + lastUpdated.toISOString().replace("T", " ").slice(0, 19) + " UTC";
  }

  function poll() {
    if (!W || !unlocked()) { state = null; render(); return; }
    W.balances(BAL_ENDPOINT).then(function (b) {
      state = b || null; lastUpdated = new Date(); render(); renderUpdated();
    }).catch(function () { state = null; render(); });
  }

  // Onboarding / unlock (7-oct-2026). No mask on this device → the rite, the one place that forges a mask properly
  // (24 words shown and CHECKED, optional lock). It used to be window.prompt + an alert() holding the 24 words.
  // A locked mask → the same unlock bar as the market and the dressing room (unlock-bar.js).
  function onUnlockClick() {
    if (!W) return;
    W.hasKeystore().then(function (has) {
      if (!has) { window.location.href = "/rito"; return; }
      var i = $("unlock-bar-pass"); if (i) i.focus();
    });
  }

  function afterChange() { render(); poll(); }
  // For wallet-send.js: refresh after a transfer, and read the last known balances (base units, strings).
  window.HlqWalletView = { refresh: function () { poll(); }, state: function () { return state; }, amount: amount };

  function bindW() {
    W = (typeof window !== "undefined" && window.HlqWallet) || null;
    if (!W) return;
    W.hasKeystore().then(function (has) {
      if (has) setBtnLabel("Abrir cartera", "Unlock wallet");
      else setBtnLabel("Conseguir mi máscara (el rito)", "Get my mask (the rite)");
      // 1-oct-2026: a mask sealed without a passphrase (the rite's default) opens by itself, as it already does in
      // the dressing room and the market. Before this, the wallet was the one place inside the villa that greeted
      // you with "—" and a button. A mask WITH a lock fails here silently and keeps the button.
      if (has && !unlocked()) W.unlock("").then(afterChange).catch(function () {});
    });
    render(); poll();
    if (!bindW._iv) bindW._iv = setInterval(poll, POLL_MS);
  }

  document.querySelectorAll(".lang button").forEach(function (b) {
    b.addEventListener("click", function () { render(); renderUpdated(); });
  });
  var ub = $("wallet-unlock");
  if (ub) ub.addEventListener("click", onUnlockClick);

  // wallet-core.js is a deferred module; bind when it signals ready (or now if already present).
  window.addEventListener("hlqwallet-ready", bindW);
  window.addEventListener("hlqwallet-changed", afterChange);   // the unlock bar opened it
  if (window.HlqWallet) bindW();
  render();
})();
