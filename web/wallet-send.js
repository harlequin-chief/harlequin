/* Harlequin — Send and Receive in the wallet (7-oct-2026).
 *
 * Until today both buttons did nothing. Send builds `Tokens.transfer(coin, to, amount)` (pallet 9, call 0),
 * signs it with the mask ON THIS DEVICE (hlq-tx.js → the same builder the market and the villa use) and hands
 * the signed bytes to the mediator, which only relays them. Receive shows the mask's name and its full address.
 *
 * The chain is the judge, not the relay: a transfer the runtime rejects (not enough funds) is still accepted
 * into the pool and still PAYS ITS FEE (measured in the lab, 7-oct). So (1) the amount is checked against the
 * balance before signing, and (2) after sending the wallet waits for the chain to INCLUDE it (our nonce moves
 * on) and then calls it done only if the RECIPIENT's balance grew by the amount; included without that =
 * rejected. (Review 7-oct: judging by our own balance alone, a fee ≥ a tiny amount looked like success.)
 * Residual, said out loud: someone else paying the same recipient that same amount in the same block would
 * read as success. The movements list (from the chain's events) is the next step.
 */
import * as tx from "/hlq-tx.js";

const PALLET_TOKENS = 9, CALL_TRANSFER = 0;
const COINS = { hlq: { idx: 0, unit: "HLQ" }, sov: { idx: 1, unit: "SOV" } };
const UNIT = 1000000000000n;                       // 12 decimals
const B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

const $ = (id) => document.getElementById(id);
const lang = () => document.documentElement.classList.contains("lang-es") ? "es" : "en";
const t = (es, en) => lang() === "es" ? es : en;
const L = (es, en) => `<span class="es" lang="es">${es}</span><span class="en" lang="en">${en}</span>`;
const esc = (x) => String(x).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const hex = (u8) => Array.from(u8, (b) => b.toString(16).padStart(2, "0")).join("");
const fromHex = (h) => Uint8Array.from(h.replace(/^0x/, "").match(/../g).map((x) => parseInt(x, 16)));

function b58decode(s) {
  let n = 0n;
  for (const c of s) { const i = B58.indexOf(c); if (i < 0) return null; n = n * 58n + BigInt(i); }
  const out = [];
  while (n > 0n) { out.unshift(Number(n & 255n)); n >>= 8n; }
  for (const c of s) { if (c !== "1") break; out.unshift(0); }
  return Uint8Array.from(out);
}

/** Parse «12,5» / «12.5» into base units. Null if not a positive amount with ≤12 decimals. */
export function parseAmount(s) {
  const v = String(s || "").trim().replace(",", ".");
  if (!/^\d+(\.\d{1,12})?$/.test(v)) return null;
  const [w, f = ""] = v.split(".");
  const x = BigInt(w) * UNIT + BigInt(f.padEnd(12, "0"));
  return x > 0n && x < (1n << 128n) ? x : null;   // u128 on the chain
}

function u128le(x) { if (x < 0n || x >= (1n << 128n)) throw new Error("amount out of range"); const b = new Uint8Array(16); for (let i = 0; i < 16; i++) { b[i] = Number(x & 255n); x >>= 8n; } return b; }

/** A recipient as the user types it → { pub: Uint8Array(32), handle } or throws a human message. */
async function resolveRecipient(input) {
  const W = window.HlqWallet, s = String(input || "").trim();
  if (/^hlq-[a-z2-7]{26}$/i.test(s)) {
    const r = await fetch("/api/resolve?handle=" + encodeURIComponent(s.toLowerCase()), { cache: "no-store" });
    if (r.status === 404) throw new Error(t("Esa máscara aún no ha pasado por la cadena, así que su nombre no se puede buscar. Pídele su dirección larga (la ve en «Recibir»).",
                                            "That mask has not touched the chain yet, so its name cannot be looked up. Ask for its long address (it is under «Receive»)."));
    if (!r.ok) throw new Error(t("No se pudo buscar ese nombre ahora mismo.", "Could not look that name up right now."));
    const pub = fromHex((await r.json()).account);
    if (W.addressOf(pub) !== s.toLowerCase()) throw new Error(t("La búsqueda devolvió otra máscara: no se envía.", "The lookup returned another mask: not sending."));
    return { pub, handle: s.toLowerCase() };
  }
  const raw = b58decode(s);
  // 2-byte prefix (1728) + 32-byte key + 2-byte checksum. Re-encoding the key must give back EXACTLY what was
  // typed: that checks the prefix (this chain) and the checksum (a typo) without a second hash library.
  if (!raw || raw.length !== 36) throw new Error(t("Eso no es ni un nombre de máscara (hlq-…) ni una dirección.", "That is neither a mask name (hlq-…) nor an address."));
  const pub = raw.slice(2, 34);
  if (W.ss58Of(pub) !== s) throw new Error(t("Esa dirección tiene un error (o es de otra red). Cópiala otra vez.", "That address has a typo (or is from another network). Copy it again."));
  return { pub, handle: W.addressOf(pub) };
}

async function nonceOf(pubHex) {
  const r = await fetch("/api/tx/meta?account=0x" + pubHex, { cache: "no-store" });
  if (!r.ok) throw new Error("meta " + r.status);
  return Number((await r.json()).nonce);
}

let gen = 0;   // one live transfer per dialog: closing or starting another one silences the old watcher

async function balanceOf(pubHex) {
  const r = await fetch("/api/balance?account=0x" + pubHex, { cache: "no-store" });
  if (!r.ok) throw new Error("balance " + r.status);
  return r.json();
}

const fmt = (x) => (window.HlqWalletView && window.HlqWalletView.amount) ? window.HlqWalletView.amount(String(x)) : String(x);

function dialog() {
  let d = $("wsend");
  if (d) return d;
  d = document.createElement("dialog");
  d.id = "wsend"; d.className = "wdlg";
  d.innerHTML = `<form method="dialog" class="wdlg-card" id="wsend-form" novalidate>
      <button type="button" class="wdlg-x" id="wsend-x" aria-label="${t("Cerrar", "Close")}">×</button>
      <h2 id="wsend-title"></h2>
      <div id="wsend-body"></div>
      <p class="wdlg-note" id="wsend-note" role="status" aria-live="polite"></p>
      <div class="wdlg-actions" id="wsend-actions"></div>
    </form>`;
  document.body.appendChild(d);
  $("wsend-x").addEventListener("click", () => d.close());
  d.addEventListener("close", () => { gen++; });
  return d;
}

function note(html, kind) { const n = $("wsend-note"); n.className = "wdlg-note" + (kind ? " " + kind : ""); n.innerHTML = html || ""; }

function openReceive() {
  const W = window.HlqWallet, d = dialog();
  const handle = W.address(), addr = W.ss58();
  $("wsend-title").innerHTML = L("Recibir", "Receive");
  $("wsend-body").innerHTML = `
    <p class="wdlg-lead">${L("Da a quien te paga cualquiera de las dos. Las dos son públicas: no abren nada.", "Give whoever pays you either one. Both are public: they open nothing.")}</p>
    <label class="wdlg-lbl">${L("Tu dirección (funciona siempre)", "Your address (always works)")}</label>
    <div class="wdlg-copy"><code id="wr-addr">${addr}</code><button type="button" class="btn-ghost" data-copy="wr-addr">${L("Copiar", "Copy")}</button></div>
    <label class="wdlg-lbl">${L("Tu nombre de máscara", "Your mask name")}</label>
    <div class="wdlg-copy"><code id="wr-handle">${handle}</code><button type="button" class="btn-ghost" data-copy="wr-handle">${L("Copiar", "Copy")}</button></div>
    <p class="wdlg-small">${L("El nombre se puede buscar en cuanto tu máscara haya hecho algo en la cadena (un envío, un puesto, una palabra en el foro). Hasta entonces, da la dirección.",
                              "The name can be looked up once your mask has done something on the chain (a transfer, a stall, a word in the forum). Until then, give the address.")}</p>`;
  $("wsend-actions").innerHTML = "";
  note("");
  d.querySelectorAll("[data-copy]").forEach((b) => b.addEventListener("click", async () => {
    try { await navigator.clipboard.writeText($(b.dataset.copy).textContent); note(L("Copiado.", "Copied."), "ok"); }
    catch (e) { note(L("No se pudo copiar: selecciónalo a mano.", "Could not copy: select it by hand."), "err"); }
  }));
  d.showModal();
}

function openSend(coinKey) {
  const d = dialog(), C = COINS[coinKey];
  $("wsend-title").innerHTML = L(`Enviar ${C.unit}`, `Send ${C.unit}`);
  const st = window.HlqWalletView && window.HlqWalletView.state();
  $("wsend-body").innerHTML = `
    <label class="wdlg-lbl" for="ws-to">${L("A quién", "To")}</label>
    <input class="wdlg-in" id="ws-to" autocomplete="off" spellcheck="false" placeholder="hlq-…  /  ${t("dirección", "address")}">
    <label class="wdlg-lbl" for="ws-amt">${L("Cuánto", "How much")} (${C.unit})</label>
    <input class="wdlg-in" id="ws-amt" inputmode="decimal" autocomplete="off" placeholder="0${t(",", ".")}00">
    <p class="wdlg-small">${L("Tienes", "You have")} ${st ? fmt(st[coinKey]) : "—"} ${C.unit}. ${L("El envío paga una pequeña comisión en HLQ.", "Sending pays a small fee in HLQ.")}</p>`;
  $("wsend-actions").innerHTML = `<button type="button" class="btn-mask" id="ws-review">${L("Revisar", "Review")}</button>`;
  note("");
  $("ws-review").addEventListener("click", () => review(coinKey));
  d.showModal();
  $("ws-to").focus();
}

async function review(coinKey) {
  const W = window.HlqWallet, C = COINS[coinKey];
  note(L("Comprobando…", "Checking…"));
  let who, amt;
  try {
    amt = parseAmount($("ws-amt").value);
    if (!amt) throw new Error(t("Pon una cantidad mayor que cero (hasta 12 decimales).", "Enter an amount above zero (up to 12 decimals)."));
    who = await resolveRecipient($("ws-to").value);
    const mine = W.pubkeyHex().replace(/^0x/, "");
    if (hex(who.pub) === mine) throw new Error(t("Esa es tu propia máscara.", "That is your own mask."));
    const bal = await balanceOf(mine);
    if (BigInt(bal[coinKey]) < amt) throw new Error(t(`No llega: tienes ${fmt(bal[coinKey])} ${C.unit}.`, `Not enough: you have ${fmt(bal[coinKey])} ${C.unit}.`));
  } catch (e) { note(esc(e.message), "err"); return; }
  $("wsend-body").innerHTML = `
    <div class="wdlg-sum">
      <div class="wdlg-big">${fmt(amt)} <span>${C.unit}</span></div>
      <div>${L("para", "to")} <code>${who.handle}</code></div>
      <div class="wdlg-small"><code>${W.ss58Of(who.pub)}</code></div>
    </div>
    <p class="wdlg-small">${L("Se firma con tu llave en este dispositivo. Una vez en la cadena no se puede deshacer.", "It is signed with your key on this device. Once on the chain it cannot be undone.")}</p>`;
  $("wsend-actions").innerHTML = `<button type="button" class="btn-ghost" id="ws-back">${L("Cambiar", "Change")}</button>
    <button type="button" class="btn-mask" id="ws-go">${L("Firmar y enviar", "Sign and send")}</button>`;
  note("");
  $("ws-back").addEventListener("click", () => openSend(coinKey));
  $("ws-go").addEventListener("click", () => send(coinKey, who, amt));
}

async function send(coinKey, who, amt) {
  const C = COINS[coinKey], go = $("ws-go"), back = $("ws-back"), my = ++gen;
  const say = (h, k) => { if (my === gen) note(h, k); };
  go.disabled = back.disabled = true;
  say(L("Firmando y enviando…", "Signing and sending…"));
  const W = await tx.openWallet();
  if (!W) { note(L("La cartera está cerrada: ábrela primero.", "The wallet is locked: open it first."), "err"); go.disabled = back.disabled = false; return; }
  const mine = W.pubkeyHex().replace(/^0x/, "");
  const to = hex(who.pub);
  let n0, r0;
  try {
    n0 = await nonceOf(mine);
    r0 = BigInt((await balanceOf(to))[coinKey]);
    const call = new Uint8Array([PALLET_TOKENS, CALL_TRANSFER, C.idx, ...who.pub, ...u128le(amt)]);
    await tx.submitCall(W, call);
  } catch (e) {
    say(L("No salió: ", "It did not go out: ") + esc(e.message || e), "err"); go.disabled = back.disabled = false; return;
  }
  say(L("Enviado. Esperando a que la cadena lo apunte…", "Sent. Waiting for the chain to record it…"));
  for (let i = 0; i < 20 && my === gen; i++) {
    await new Promise((r) => setTimeout(r, 3000));
    let n, r1;
    try { n = await nonceOf(mine); r1 = BigInt((await balanceOf(to))[coinKey]); } catch (e) { continue; }
    if (n <= n0) continue;                                   // not included yet
    window.HlqWalletView && window.HlqWalletView.refresh();
    if (r1 - r0 >= amt) {
      say(L(`Hecho: ${fmt(amt)} ${C.unit} enviados a ${who.handle}.`, `Done: ${fmt(amt)} ${C.unit} sent to ${who.handle}.`), "ok");
      if (my === gen) {
        $("wsend-actions").innerHTML = `<button type="button" class="btn-mask" id="ws-ok">${L("Cerrar", "Close")}</button>`;
        $("ws-ok").addEventListener("click", () => $("wsend").close());
      }
    } else {
      say(L("La cadena lo rechazó y solo se cobró la comisión. Revisa tu saldo antes de repetirlo.", "The chain rejected it and only the fee was charged. Check your balance before trying again."), "err");
      if (my === gen) go.disabled = back.disabled = false;
    }
    return;
  }
  say(L("Aún no aparece en la cadena. Mira tu saldo en un minuto antes de repetirlo.", "It is not on the chain yet. Check your balance in a minute before repeating it."), "err");
  window.HlqWalletView && window.HlqWalletView.refresh();
}

function wire() {
  document.querySelectorAll(".purse").forEach((p) => {
    const coinKey = p.classList.contains("sov") ? "sov" : "hlq";
    const [s, r] = p.querySelectorAll(".p-actions button");
    if (s) s.addEventListener("click", () => openSend(coinKey));
    if (r) r.addEventListener("click", openReceive);
  });
}
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", wire); else wire();
