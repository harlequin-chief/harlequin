/* Harlequin — market board ("La Plaza").  #613 Level A  [ES MODULE]
 *
 * An offers BOARD, not an exchange. Each offer is a content-addressed listing a mask published on-chain;
 * the chain holds only { id, offerer pubkey, detail_hash (32B), category, withdrawn, hidden } and the detail
 * body (item, terms, price-if-any — free text) lives off-chain on IPFS.
 *
 * The client TRUSTS NOTHING the feed says:
 *   - it DERIVES the handle from the pubkey (never the feed's `handle` field) — a compromised feed cannot
 *     paint a foreign handle over a pubkey;
 *   - it fetches the detail from its OWN IPFS gateway and requires sha256(body) == the on-chain detail_hash
 *     (the authoritative anchor). cidV1FromDigest(detail_hash) == detail_cid is a secondary consistency check.
 *   - any failure → the offer is shown but badged UNVERIFIED, and an unverifiable detail body is never shown.
 *
 * Read-only & mediated (§83): the feed is the panel's own backend (/api/market); never a raw node RPC, never
 * a third-party IPFS gateway (which would leak which offers a viewer reads). Publishing signs client-side +
 * submits via the shared tx layer (#696/#616) — the button is disabled here until that piece lands.
 */
import { toHex, cidV1FromDigest, deriveHandle, fromHex, verifyBody } from "./content-id.js";
import * as tx from "./hlq-tx.js";
import { markInto, prefetch } from "./justice-marks.js";

const MARKET_URL = "/api/market/offers";  // same-origin mediator (#613, live 2026-07-20)
const IPFS_GATEWAY = "/api/ipfs/";        // same-origin serve-only relay (reader IP never leaves us)
const POLL_MS = 15000;
const $ = (id) => document.getElementById(id);
const lang = () => document.documentElement.classList.contains("lang-es") ? "es" : "en";
const t = (es, en) => lang() === "es" ? es : en;

let lastUpdated = null;
let lastOffers = [];

function badge(state) {
  const b = document.createElement("span");
  b.className = "offer-badge " + state;
  b.textContent = state === "ok" ? t("verificada", "verified")
    : state === "bad" ? t("no verificada", "unverified")
    : state === "missing" ? t("detalle no disponible", "detail unavailable")
    : t("verificando…", "verifying…");
  return b;
}

function span(cls, text) {
  const s = document.createElement("span");
  s.className = cls;
  if (text != null) s.textContent = text;
  return s;
}

// Build a card for one offer, then asynchronously verify it (derive handle, fetch+hash the IPFS detail).
function offerCard(o) {
  const card = document.createElement("article");
  card.className = "offer";
  // D3 (7-oct-2026): market-deals.js arms «Cerrar trato» from these — the offer id and its owner's key.
  if (Number.isSafeInteger(o.id)) card.dataset.offer = String(o.id);
  if (o.pubkey) card.dataset.pubkey = String(o.pubkey).startsWith("0x") ? o.pubkey : "0x" + o.pubkey;

  const head = document.createElement("div");
  head.className = "offer-head";
  const handleEl = span("offer-handle", o.handle || "hlq-…");   // replaced by the DERIVED handle below
  head.appendChild(handleEl);
  markInto(head, o.pubkey, lang);   // moderation-B: a guilty verdict is SHOWN next to the offer, never hides it
  if (o.category) head.appendChild(span("offer-cat", o.category));
  const badgeEl = badge("pending");
  head.appendChild(badgeEl);
  card.appendChild(head);

  const detailEl = document.createElement("p");
  detailEl.className = "offer-detail";
  detailEl.textContent = t("cargando el detalle…", "loading detail…");
  card.appendChild(detailEl);

  // Talk to the seller (2026-09-26): the forum is already signed, feeless and verified end to end, so the
  // first way two strangers talk about an offer is a PUBLIC thread there, prefilled with the offer reference.
  // Private chat (Corro) is a later layer; nothing here pretends otherwise.
  if (Number.isSafeInteger(o.id) && o.id >= 0) {
    const foot = document.createElement("div");
    foot.className = "offer-foot";
    const ask = document.createElement("a");
    ask.className = "offer-ask";
    ask.href = "/el-foro?sobre=oferta-" + o.id;
    ask.textContent = t("Preguntar en el foro (público) →", "Ask in the forum (public) →");
    foot.appendChild(ask);
    card.appendChild(foot);
  }

  verifyOffer(o, handleEl, badgeEl, detailEl);
  return card;
}

async function verifyOffer(o, handleEl, badgeEl, detailEl) {
  let ok = true;
  // 1) Handle is DERIVED from the pubkey; never trust the feed's handle.
  try {
    const derived = await deriveHandle(fromHex(o.pubkey));
    handleEl.textContent = derived;                 // always display the derived handle
    if (o.handle && o.handle !== derived) ok = false; // feed lied about the handle
  } catch (e) { ok = false; handleEl.textContent = "hlq-?"; }

  // 2) Detail integrity: fetch from OWN gateway, require sha256(body) == on-chain detail_hash.
  if (!IPFS_GATEWAY) {
    detailEl.textContent = t("detalle no disponible (sin gateway propio configurado)",
                             "detail unavailable (no own gateway configured)");
    badgeEl.replaceWith(badge(ok ? "pending" : "bad"));
    return;
  }
  // 1-oct-2026: «nobody can serve the detail right now» is not «the detail was tampered with». A missing body is
  // shown neutral (gray, nothing rendered); red stays for a body that does NOT match the chain.
  let missing = false;
  try {
    if (o.detail_stored === false) { missing = true; throw new Error("missing"); }   // the relay says it holds no copy
    const res = await fetch(IPFS_GATEWAY + encodeURIComponent(o.detail_cid), { cache: "no-store" });
    if (res.status === 404 || res.status === 502 || res.status === 503) { missing = true; throw new Error("missing"); }
    if (!res.ok) throw new Error("http " + res.status);
    const body = new Uint8Array(await res.arrayBuffer());
    const bodyOk = await verifyBody(body, o.detail_hash);            // AUTHORITATIVE check
    // secondary consistency: the cid the feed gave must derive from the on-chain hash
    const cidOk = cidV1FromDigest(fromHex(o.detail_hash)) === o.detail_cid;
    ok = ok && bodyOk && cidOk;
    detailEl.textContent = bodyOk
      ? new TextDecoder().decode(body)                                // verified body, rendered as text
      : t("detalle NO coincide con el hash de la cadena — no se muestra",
          "detail does NOT match the on-chain hash — not shown");
  } catch (e) {
    ok = false;
    detailEl.textContent = missing
      ? t("El detalle de esta oferta no está disponible ahora mismo (nadie lo sirve). La oferta sigue en la cadena.",
          "This offer's detail is not available right now (nobody is serving it). The offer is still on the chain.")
      : t("no se pudo verificar el detalle", "could not verify the detail");
  }
  badgeEl.replaceWith(badge(ok ? "ok" : missing ? "missing" : "bad"));
}

// 2026-09-25 (the operator): (1) a read failure used to WIPE the board and show "empty" — it lied and threw
// away what was already shown; now it is kept and stated. (2) the API paginates 50 at a time (next_cursor) and
// this was being ignored: past the first 50, the rest vanished without notice; now "See more" asks for the
// next page (on demand, not on every poll: rate-limited per IP).
let readFailed = false, lastBest = null, total = null, nextCursor = null, extraOffers = [];
function status() {
  const s = $("board-status");
  if (!s) return;
  if (readFailed && lastBest == null) {
    s.className = "post-note board-status bad";
    s.textContent = t("No se ha podido leer el tablón de la cadena ahora mismo. Se reintenta solo cada 15 s.",
                      "Could not read the board from the chain right now. It retries by itself every 15 s.");
    return;
  }
  if (lastBest == null) { s.textContent = t("Leyendo la cadena…", "Reading the chain…"); return; }
  const shown = lastOffers.length + extraOffers.length, n = total == null ? shown : total;
  s.className = "post-note board-status" + (readFailed ? " bad" : "");
  s.textContent = t(`${n} oferta${n === 1 ? "" : "s"}${n > shown ? ` (se ven ${shown})` : ""} · leído de la cadena en el bloque #${lastBest}`,
                    `${n} offer${n === 1 ? "" : "s"}${n > shown ? ` (${shown} shown)` : ""} · read from the chain at block #${lastBest}`)
    + (readFailed ? t(" (la última lectura falló; se reintenta)", " (the last read failed; retrying)") : "");
}

function render() {
  const list = $("board-list"), empty = $("board-empty");
  if (!list) return;
  status();
  const more = $("board-more-wrap"); if (more) more.hidden = nextCursor == null;
  // hide withdrawn/hidden — append-only on-chain, but a board shows neither.
  const offers = lastOffers.concat(extraOffers).filter((o) => !o.withdrawn && !o.hidden);
  list.textContent = "";
  // (`hidden`, not style="…": the CSP style-src 'self' ignores inline styles)
  if (!offers.length) { if (empty) empty.hidden = lastBest == null; return; }
  if (empty) empty.hidden = true;
  prefetch(offers.map((o) => o.pubkey));
  const frag = document.createDocumentFragment();
  offers.forEach((o) => frag.appendChild(offerCard(o)));
  list.appendChild(frag);
}

function renderUpdated() {
  const el = $("updated");
  if (!el || !lastUpdated) return;
  el.textContent = t("actualizado ", "updated ") + lastUpdated.toISOString().replace("T", " ").slice(0, 19) + " UTC";
}

function poll() {
  if (!MARKET_URL) { lastOffers = []; render(); return; }   // honest empty board pre-launch
  fetch(MARKET_URL, { cache: "no-store" })
    .then((r) => { if (!r.ok) throw new Error("http " + r.status); return r.json(); })
    .then((data) => {
      lastOffers = (data && data.offers) || [];
      lastUpdated = data && data.updated_at ? new Date(data.updated_at) : new Date();
      lastBest = data ? data.best_block : null; total = data && typeof data.total === "number" ? data.total : null;
      if (!extraOffers.length) nextCursor = data ? data.next_cursor : null;   // with extra pages open, the latest one wins
      readFailed = false;
      render(); renderUpdated();
    })
    .catch(() => { readFailed = true; status(); });
}

function loadMore() {
  if (nextCursor == null) return;
  fetch(MARKET_URL + "?cursor=" + encodeURIComponent(nextCursor), { cache: "no-store" })
    .then((r) => { if (!r.ok) throw new Error("http " + r.status); return r.json(); })
    .then((data) => {
      const seen = new Set(lastOffers.concat(extraOffers).map((o) => o.id));
      extraOffers = extraOffers.concat(((data && data.offers) || []).filter((o) => !seen.has(o.id)));
      nextCursor = data ? data.next_cursor : null;
      render();
    })
    .catch(() => { readFailed = true; status(); });
}

// Re-localise on language toggle (app.js handles the static spans).
document.querySelectorAll(".lang button").forEach((b) => {
  b.addEventListener("click", () => { render(); renderUpdated(); });
});

// ── Publishing (2026-09-23): the mask signs HERE, the mediator only relays ─────────────────────────
// The text goes to the content store (re-hashed here), then Market.publish(detail, category) is signed
// on-device and relayed — the shared path in hlq-tx.js (the forum uses the same). Feeless for a new mask.
const PALLET_MARKET = 13, CALL_PUBLISH = 0;
const MAX_CATEGORY_BYTES = 16;       // runtime: MaxCategoryLen = ConstU32<16>

function note(kind, es, en) {
  const el = $("of-note");
  if (!el) return;
  el.className = "post-note " + kind;
  el.textContent = t(es, en);
}

async function publishOffer() {
  const btn = $("of-publish");
  const W = await tx.openWallet();
  if (!W) {
    return note("bad", "No hay máscara abierta en este navegador. Fórjala en el rito (/rito) y vuelve.",
                       "No mask is open in this browser. Forge one in the rite (/rito) and come back.");
  }
  const detailText = ($("of-detail").value || "").trim();
  const category = ($("of-category").value || "").trim();
  const catBytes = new TextEncoder().encode(category);
  if (!detailText) return note("bad", "Escribe qué ofreces.", "Write what you offer.");
  if (catBytes.length > MAX_CATEGORY_BYTES) return note("bad", "La categoría admite 16 bytes como mucho.", "The category allows 16 bytes at most.");

  btn.disabled = true;
  try {
    note("info", "Guardando el texto con su huella…", "Storing the text under its fingerprint…");
    const hash = await tx.storeText(detailText);
    note("info", "Tu máscara firma en este dispositivo…", "Your mask signs on this device…");
    // 7-oct-2026: the runtime says where Market and `publish` are (a new runtime may move them); 13/0 is the fallback.
    let pi = PALLET_MARKET, ci = CALL_PUBLISH;
    try { const c = await (await fetch("/api/capabilities", { cache: "no-store" })).json();
          const m = c && c.pallets && c.pallets.Market; if (m && Number.isInteger(m.index) && Number.isInteger(m.calls.publish)) { pi = m.index; ci = m.calls.publish; } } catch (e) {}
    const call = new Uint8Array([pi, ci, ...hash, ...tx.compact(catBytes.length), ...catBytes]);
    const h = await tx.submitCall(W, call);
    note("good", "Publicada (tx " + h.slice(0, 12) + "…). Aparecerá en el tablón en unos segundos.",
                 "Published (tx " + h.slice(0, 12) + "…). It will appear on the board in a few seconds.");
    $("of-detail").value = ""; $("of-category").value = "";
    setTimeout(poll, 13000);
  } catch (e) {
    const m = String(e && e.message || e);
    if (m === "too long") note("bad", "El detalle es demasiado largo.", "The detail is too long.");
    else note("bad", "No se pudo publicar: " + m, "Could not publish: " + m);
  } finally { btn.disabled = false; }
}

const pubBtn = $("of-publish");
if (pubBtn) {
  pubBtn.addEventListener("click", publishOffer);
  // Needs a wallet build that can give its SS58 address and sign an extrinsic payload. An older wasm
  // (measured 2026-09-23: production served the 15-jul build, without either) keeps the button OFF.
  const arm = () => { pubBtn.disabled = !tx.capable(); };
  window.addEventListener("hlqwallet-ready", arm);
  arm();
}

poll();
if (MARKET_URL) setInterval(poll, POLL_MS);
document.addEventListener("click", (ev) => { if (ev.target.closest("#board-more")) loadMore(); });

// 7-oct-2026: the line above the board invites visitors to enter the villa; a neighbour already inside does not need it.
const hideInvite = () => { const W = window.HlqWallet, bn = document.querySelector(".preview-banner"); if (bn && W && W.isUnlocked()) bn.hidden = true; };
window.addEventListener("hlqwallet-ready", hideInvite);
window.addEventListener("hlqwallet-changed", hideInvite);
hideInvite();
