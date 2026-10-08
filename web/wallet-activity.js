/* Harlequin — the wallet's "Activity" (#1531, 7-oct-2026).
 *
 * Reads /api/transfers (the mediator's cache of the chain's Tokens events, FINALIZED blocks only) for the open mask
 * and lists them newest first: received / sent / fee, the amount, the other mask and the block. Public chain data:
 * nothing here is secret and nothing is signed. The list trails the chain by the finality lag (~3 min); a transfer
 * just made appears when its block is final, and the line under the list says so while it is pending.
 */
(function () {
  "use strict";
  var $ = function (id) { return document.getElementById(id); };
  function es() { return document.documentElement.classList.contains("lang-es"); }
  function t(a, b) { return es() ? a : b; }
  var POLL_MS = 20000, PAGE = 30;
  var items = [], more = null, info = null, busy = false;

  function hexToBytes(h) {
    h = h.replace(/^0x/, "");
    var u = new Uint8Array(h.length / 2);
    for (var i = 0; i < u.length; i++) u[i] = parseInt(h.substr(i * 2, 2), 16);
    return u;
  }
  function handleOf(hex) {
    try { return window.HlqWallet.addressOf(hexToBytes(hex)); } catch (e) { return hex.slice(0, 10) + "…"; }
  }
  function fmt(v) {
    var A = window.HlqWalletView && window.HlqWalletView.amount;
    return A ? A(String(v)) : String(v);
  }
  function me() {
    var W = window.HlqWallet;
    return W && W.isUnlocked && W.isUnlocked() ? W.pubkeyHex() : null;
  }

  function el(tag, cls, txt) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (txt != null) e.textContent = txt;
    return e;
  }

  function render() {
    var box = $("tx-rows"), empty = $("tx-empty");
    if (!box) return;
    box.textContent = "";
    var self = me();
    items.forEach(function (x) {
      var row = el("div", "tx-row " + x.dir + (x.kind === "fee" ? " fee" : ""));
      var label = x.kind === "fee" ? (x.dir === "in" ? t("Comisión cobrada", "Fee earned") : t("Comisión", "Fee"))
                                   : (x.dir === "in" ? t("Recibido", "Received") : t("Enviado", "Sent"));
      row.appendChild(el("span", "tx-dir", label));
      var coin = x.coin === "sov" ? "SOV" : "HLQ";
      row.appendChild(el("span", "tx-amt", (x.dir === "in" ? "+ " : "− ") + fmt(x.amount) + " " + coin));
      var other = x.other && x.other.toLowerCase() !== (self || "").toLowerCase() ? handleOf(x.other) : "";
      var cp = el("span", "tx-cp", other ? (x.dir === "in" ? t("de ", "from ") : t("a ", "to ")) + other : "");
      if (other) cp.title = other;
      row.appendChild(cp);
      row.appendChild(el("span", "tx-h", "#" + Number(x.block).toLocaleString(es() ? "es-ES" : "en-US")));
      box.appendChild(row);
    });
    if (more) {
      var b = el("button", "btn-ghost tx-more", t("Ver más antiguos", "Show older"));
      b.type = "button";
      b.addEventListener("click", function () { load(more); });
      box.appendChild(b);
    }
    if (empty) empty.style.display = items.length ? "none" : "";
    var st = $("tx-state");
    if (!st) { st = el("p", "tx-state"); st.id = "tx-state"; box.parentNode.appendChild(st); }
    st.textContent = !info ? "" : info.complete
      ? t("Al día con la cadena hasta el bloque #", "Up to date with the chain to block #") + info.indexed_to.toLocaleString(es() ? "es-ES" : "en-US") +
        t(". Lo último aparece al confirmarse su bloque (unos minutos).", ". The latest shows once its block is confirmed (a few minutes).")
      : t("Leyendo la cadena: bloque #", "Reading the chain: block #") + info.indexed_to.toLocaleString(es() ? "es-ES" : "en-US") +
        t(" de #", " of #") + info.head.toLocaleString(es() ? "es-ES" : "en-US") + t(". La lista puede estar incompleta.", ". The list may be incomplete.");
  }

  function load(before) {
    var acc = me();
    if (!acc || busy) { if (!acc) { items = []; more = null; info = null; render(); } return; }
    busy = true;
    var url = "/api/transfers?account=" + acc + "&limit=" + PAGE + (before ? "&offset=" + before : "");
    fetch(url, { cache: "no-store" }).then(function (r) { return r.ok ? r.json() : null; }).then(function (j) {
      busy = false;
      if (!j || !Array.isArray(j.items)) return;
      items = before ? items.concat(j.items) : j.items;
      more = j.next_offset;
      info = { complete: !!j.complete, indexed_to: j.indexed_to | 0, head: j.head | 0 };
      render();
    }).catch(function () { busy = false; });
  }

  function refresh() { if (!more || items.length <= PAGE) load(null); else render(); }

  window.addEventListener("hlqwallet-ready", function () { load(null); });
  window.addEventListener("hlqwallet-changed", function () { load(null); });
  document.querySelectorAll(".lang button").forEach(function (b) { b.addEventListener("click", render); });
  setInterval(refresh, POLL_MS);
  if (window.HlqWallet) load(null);
})();
