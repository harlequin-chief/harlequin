/* Harlequin — closing a deal in the market (D3, 7-oct-2026).  [ES MODULE]
 *
 * A deal is two masks both saying, on chain, «this happened» over an offer: the buyer proposes it from the offer
 * (Market.propose_deal), the seller confirms it (Market.confirm_deal) and from then on it is CLOSED for good; the
 * proposer may withdraw it while unconfirmed (cancel_deal). The chain moves no money here: paying is a separate
 * transfer from the wallet. The pallet/call indices come from the runtime's own metadata (/api/capabilities): on a
 * chain without deals this module does nothing at all — no button promises what the chain cannot take.
 */
import * as tx from "./hlq-tx.js";

const $ = (id) => document.getElementById(id);
const lang = () => document.documentElement.classList.contains("lang-es") ? "es" : "en";
const t = (es, en) => lang() === "es" ? es : en;
const POLL_MS = 15000;

let caps = null, deals = [], busy = false;
// The board re-renders its cards every 15 s: the two-step state and the note of each offer live HERE, by offer id,
// and are put back on the fresh card — otherwise a poll would silently undo «Sí, proponer» and erase the outcome.
const offerState = new Map();   // offer id → { armed, cls, text }

function u64le(n) {
  const b = new Uint8Array(8); let x = BigInt(n);
  for (let i = 0; i < 8; i++) { b[i] = Number(x & 0xffn); x >>= 8n; }
  return b;
}
function hexToBytes(h) {
  h = String(h).replace(/^0x/, "");
  const u = new Uint8Array(h.length / 2);
  for (let i = 0; i < u.length; i++) u[i] = parseInt(h.substr(i * 2, 2), 16);
  return u;
}
function handleOf(hex) { try { return window.HlqWallet.addressOf(hexToBytes(hex)); } catch (e) { return String(hex).slice(0, 10) + "…"; } }
function me() { const W = window.HlqWallet; return W && W.isUnlocked && W.isUnlocked() ? W.pubkeyHex() : null; }
function el(tag, cls, text) { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; }

function call(name, args) {
  const m = caps.pallets.Market;
  return new Uint8Array([m.index, m.calls[name], ...args]);
}

// `say(cls, text)` writes the outcome wherever it must show (a card's note survives re-renders through offerState).
async function send(name, args, say, okEs, okEn) {
  if (busy) return;
  const W = await tx.openWallet();
  if (!W) { say("bad", t("Abre tu máscara para firmar.", "Open your mask to sign.")); return; }
  busy = true;
  say("info", t("Tu máscara firma en este dispositivo…", "Your mask signs on this device…"));
  try {
    const before = JSON.stringify(deals.map((d) => [d.id, d.closed]));
    await tx.submitCall(W, call(name, args));
    say("info", t("Enviado. Esperando a que la cadena lo apunte…", "Sent. Waiting for the chain to record it…"));
    // «Hecho» only when the chain shows the change — never because the relay accepted the bytes.
    for (let i = 0; i < 12; i++) {
      await new Promise((r) => setTimeout(r, 5000));
      await load();
      if (JSON.stringify(deals.map((d) => [d.id, d.closed])) !== before) {
        say("good", t(okEs, okEn)); return;
      }
    }
    say("bad", t("La cadena no lo ha apuntado en un minuto: puede que lo rechazara. Revisa «Tus tratos».",
                 "The chain has not recorded it within a minute: it may have rejected it. Check «Your deals»."));
  } catch (e) {
    say("bad", t("No se pudo enviar: ", "Could not send: ") + String(e && e.message || e));
  } finally { busy = false; render(); }
}

// One button per offer card that is NOT mine: «Cerrar trato» (buyer → seller).
function openProposal(offerId, self) {
  return deals.some((d) => d.offer === offerId && !d.closed && !d.expired && d.proposer.toLowerCase() === self);
}

function armCards() {
  const self = (me() || "").toLowerCase();
  document.querySelectorAll("#board-list article.offer[data-offer]").forEach((card) => {
    const owner = (card.dataset.pubkey || "").toLowerCase();
    const id = Number(card.dataset.offer);
    const btn = card.querySelector(".deal-propose");
    // Nothing to propose with a closed mask, on your OWN offer, or while your proposal on it is still open.
    // Re-evaluated on every call: buttons set up before the mask opened must not survive on your own offers.
    const want = !!self && !!owner && owner !== self && !openProposal(id, self);
    if (btn && !want) {
      btn.remove();
      if (openProposal(id, self)) {
        const n = card.querySelector(".deal-note");
        if (n && !(offerState.get(id) || {}).text) { n.className = "post-note deal-note info"; n.textContent = t("Tu propuesta está pendiente: la ves en «Tus tratos».", "Your proposal is pending: see «Your deals»."); }
      } else { const n = card.querySelector(".deal-note"); if (n) n.remove(); }
      return;
    }
    if (btn || !want) return;
    const foot = card.querySelector(".offer-foot") || card.appendChild(el("div", "offer-foot"));
    const st = offerState.get(id) || {};
    const b = el("button", "btn-ghost deal-propose", st.armed ? t("Sí, proponer el trato", "Yes, propose the deal")
                                                              : t("Cerrar trato con esta máscara", "Close a deal with this mask"));
    b.type = "button";
    const note = el("p", "post-note deal-note" + (st.cls ? " " + st.cls : ""), st.text || "");
    const say = (cls, text) => {
      offerState.set(id, Object.assign(offerState.get(id) || {}, { cls, text }));
      const n = document.querySelector(`#board-list article.offer[data-offer='${id}'] .deal-note`);
      if (n) { n.className = "post-note deal-note " + cls; n.textContent = text; }
    };
    // Two steps, in the card (no browser dialogs): the first click says what it means, the second signs.
    b.addEventListener("click", () => {
      const cur = offerState.get(id) || {};
      if (!cur.armed) {
        offerState.set(id, Object.assign(cur, { armed: true }));
        b.textContent = t("Sí, proponer el trato", "Yes, propose the deal");
        say("info", t("Quedará en la cadena que propones cerrar el trato de esta oferta con esta máscara; ella tendrá que confirmarlo. El pago va aparte, desde la cartera.",
                      "The chain will record that you propose to close the deal on this offer with this mask; they will have to confirm it. Payment is separate, from the wallet."));
        return;
      }
      offerState.set(id, Object.assign(cur, { armed: false }));
      send("propose_deal", [...u64le(id), ...hexToBytes(owner)], say,
           "Propuesta apuntada. Falta que la otra máscara la confirme.", "Proposal recorded. The other mask still has to confirm it.");
    });
    foot.appendChild(b); card.appendChild(note);
  });
}

const rowState = new Map();     // deal id → { cls, text }: «Tus tratos» re-renders too
function rowSay(id) {
  return (cls, text) => {
    rowState.set(id, { cls, text });
    const n = document.querySelector(`#deals-list .deal-note[data-deal='${id}']`);
    if (n) { n.className = "post-note deal-note " + cls; n.textContent = text; }
  };
}

function render() {
  const box = $("deals"), list = $("deals-list");
  if (!box || !list) return;
  const self = (me() || "").toLowerCase();
  box.hidden = !caps || !caps.deals || !self;
  if (box.hidden) return;
  list.textContent = "";
  if (!deals.length) { list.appendChild(el("p", "book-empty", t("Aún no tienes tratos. Se proponen desde una oferta de otra máscara.", "No deals yet. They are proposed from another mask's offer."))); return; }
  deals.forEach((d) => {
    const row = el("div", "deal-row" + (d.closed ? " closed" : d.expired ? " expired" : ""));
    const iAmProposer = d.proposer.toLowerCase() === self;
    const other = iAmProposer ? d.counterparty : d.proposer;
    row.appendChild(el("span", "deal-what", t("Oferta #", "Offer #") + d.offer));
    row.appendChild(el("span", "deal-who", (iAmProposer ? t("con ", "with ") : t("de ", "from ")) + handleOf(other)));
    const state = d.closed ? t("Cerrado", "Closed") : d.expired ? t("Caducado sin confirmar", "Expired unconfirmed")
      : iAmProposer ? t("Esperando a la otra máscara", "Waiting for the other mask") : t("Te toca confirmar", "Yours to confirm");
    row.appendChild(el("span", "deal-state", state));
    const act = el("span", "deal-act");
    const rs = rowState.get(d.id);
    const note = el("p", "post-note deal-note" + (rs ? " " + rs.cls : ""), rs ? rs.text : "");
    note.dataset.deal = String(d.id);
    if (!d.closed && !d.expired && !iAmProposer) {
      const b = el("button", "btn-gold", t("Confirmar trato", "Confirm deal")); b.type = "button";
      b.addEventListener("click", () => send("confirm_deal", [...u64le(d.id)], rowSay(d.id), "Trato cerrado y apuntado en la cadena.", "Deal closed and recorded on the chain."));
      act.appendChild(b);
    }
    if (!d.closed && iAmProposer) {
      const b = el("button", "btn-ghost", t("Retirar", "Withdraw")); b.type = "button";
      b.addEventListener("click", () => send("cancel_deal", [...u64le(d.id)], rowSay(d.id), "Propuesta retirada.", "Proposal withdrawn."));
      act.appendChild(b);
    }
    row.appendChild(act);
    list.appendChild(row); list.appendChild(note);
  });
}

async function load() {
  const acc = me();
  if (!acc) { deals = []; render(); return; }
  try {
    const r = await fetch("/api/market/deals?account=" + acc, { cache: "no-store" });
    if (!r.ok) return;
    const j = await r.json();
    deals = Array.isArray(j.deals) ? j.deals : [];
    render(); armCards();
  } catch (e) { /* keep what is shown */ }
}

async function start() {
  try {
    const r = await fetch("/api/capabilities", { cache: "no-store" });
    caps = r.ok ? await r.json() : null;
  } catch (e) { caps = null; }
  if (!caps || !caps.deals || !caps.pallets || !caps.pallets.Market) return;   // this chain takes no deals: stay silent
  const list = $("board-list");
  if (list) new MutationObserver(armCards).observe(list, { childList: true });
  armCards(); load();
  window.addEventListener("hlqwallet-ready", () => { armCards(); load(); });
  window.addEventListener("hlqwallet-changed", () => { armCards(); load(); });
  setInterval(load, POLL_MS);
}
start();
