/* Harlequin — El Foro, live (2026-09-23).  [ES MODULE]
 *
 * Reads the on-chain post index through the mediator (/api/forum/posts) and TRUSTS NOTHING it says:
 *   - the author's handle is DERIVED here from the pubkey;
 *   - each body is fetched from our own gateway and must hash to the on-chain fingerprint, or it is not
 *     shown (a server that changes one comma is caught by every reader).
 * Writing: the text goes to the content store, then Forum.post(body, parent) is signed with the mask ON
 * THIS DEVICE and relayed (hlq-tx.js, the same path as the market). Feeless for a new mask.
 * Moderation-B (the maintainer, 2026-07-15): nothing is ever hidden here — jury verdicts label conduct elsewhere and
 * each reader decides. Bodies are rendered as TEXT (textContent), never as HTML.
 */
import { deriveHandle, fromHex, verifyBody } from "./content-id.js";
import * as tx from "./hlq-tx.js";
import { markInto, prefetch } from "./justice-marks.js";

const FEED = "/api/forum/posts";
const GATEWAY = "/api/ipfs/";
const PALLET_FORUM = 11, CALL_POST = 0;
const POLL_MS = 15000;
const $ = (id) => document.getElementById(id);
const lang = () => document.documentElement.classList.contains("lang-es") ? "es" : "en";
const t = (es, en) => lang() === "es" ? es : en;

let posts = [];
let replyTo = null;
const bodyCache = new Map();   // body_hash → {ok, text} (a verified body never changes: content-addressed)

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

async function verifiedBody(p) {
  if (bodyCache.has(p.body_hash)) return bodyCache.get(p.body_hash);
  let out = { ok: false, text: t("no se pudo verificar el texto", "could not verify the text") };
  try {
    // 7-oct-2026: the relay tells us in the list when it holds no copy — then we do not ask (no failed request).
    const r = p.body_stored === false ? { status: 404 } : await fetch(GATEWAY + encodeURIComponent(p.body_cid), { cache: "no-store" });
    // 1-oct-2026: an unavailable body is not a tampered one — say so, neutral.
    if (r.status === 404 || r.status === 502 || r.status === 503) {
      out = { ok: false, missing: true, text: t("El texto de este mensaje no está disponible ahora mismo (nadie lo sirve). El mensaje sigue en la cadena.",
                                                "This message's text is not available right now (nobody is serving it). The message is still on the chain.") };
      return out;
    }
    if (!r.ok) throw new Error("http " + r.status);
    const body = new Uint8Array(await r.arrayBuffer());
    if (await verifyBody(body, p.body_hash)) out = { ok: true, text: new TextDecoder().decode(body) };
    else out = { ok: false, text: t("el texto NO coincide con la huella de la cadena — no se muestra",
                                    "the text does NOT match the on-chain fingerprint — not shown") };
  } catch (e) { /* keep the honest failure */ }
  if (out.ok) bodyCache.set(p.body_hash, out);
  return out;
}

function card(p, depth) {
  const a = el("article", "post live" + (depth ? " post-reply" : ""));
  a.dataset.id = String(p.id);
  if (depth > 1) a.classList.add("post-deep");
  const head = el("div", "post-head");
  const who = el("code", "post-mask", "hlq-…");
  const meta = el("span", "post-meta", t("verificando…", "verifying…"));
  head.append(who, el("span", "post-sep", " · "), meta);
  markInto(head, p.pubkey, lang);   // moderation-B: a guilty verdict is SHOWN, never used to hide
  const body = el("p", "post-body", "…");
  const foot = el("div", "post-foot");
  const reply = el("button", "post-reply-btn", t("Responder", "Reply"));
  reply.type = "button";
  reply.dataset.reply = String(p.id);
  foot.append(el("span", "post-cid-k", "#" + p.id + " · " + t("bloque ", "block ") + p.at_block), reply);
  a.append(head, body, foot);
  deriveHandle(fromHex(p.pubkey)).then((h) => { who.textContent = h; }).catch(() => { who.textContent = "hlq-?"; });
  verifiedBody(p).then((b) => {
    body.textContent = b.text;
    meta.textContent = b.ok ? t("verificado contra la cadena", "verified against the chain")
      : b.missing ? t("texto no disponible", "text unavailable") : t("NO verificado", "NOT verified");
    meta.classList.add(b.ok ? "ok" : b.missing ? "missing" : "bad");
  });
  return a;
}

// 2026-09-25 (the operator): paginate by THREADS (a thread = root + its replies, never split) and always say
// where what is shown comes from: a plain "empty" does not distinguish "there is nothing" from "the chain
// could not be read".
const PAGE = 20;
let shownThreads = PAGE, lastRead = null, readFailed = false;
function status() {
  const s = $("forum-status");
  if (!s) return;
  if (readFailed && !lastRead) {
    s.className = "post-note forum-status bad";
    s.textContent = t("No se ha podido leer el foro de la cadena ahora mismo. Se reintenta solo cada 15 s.",
                      "Could not read the forum from the chain right now. It retries by itself every 15 s.");
    return;
  }
  if (!lastRead) { s.textContent = t("Leyendo la cadena…", "Reading the chain…"); return; }
  const n = posts.length, th = posts.filter((p) => p.parent == null).length;
  s.className = "post-note forum-status" + (readFailed ? " bad" : "");
  s.textContent = t(`${n} mensaje${n === 1 ? "" : "s"} en ${th} hilo${th === 1 ? "" : "s"} · leído de la cadena en el bloque #${lastRead.best}`,
                    `${n} message${n === 1 ? "" : "s"} in ${th} thread${th === 1 ? "" : "s"} · read from the chain at block #${lastRead.best}`)
    + (readFailed ? t(" (la última lectura falló; se reintenta)", " (the last read failed; retrying)") : "");
}

function render() {
  const list = $("forum-list"), empty = $("forum-empty");
  if (!list) return;
  list.textContent = "";
  status();
  const more = $("forum-more-wrap");
  if (!posts.length) { if (empty) empty.hidden = !lastRead; if (more) more.hidden = true; return; }
  if (empty) empty.hidden = true;
  prefetch(posts.map((p) => p.pubkey));
  const kids = new Map();
  for (const p of posts) if (p.parent != null) {
    if (!kids.has(p.parent)) kids.set(p.parent, []);
    kids.get(p.parent).push(p);
  }
  for (const arr of kids.values()) arr.sort((x, y) => x.id - y.id);          // replies: oldest first
  const frag = document.createDocumentFragment();
  const walk = (p, depth) => {
    frag.appendChild(card(p, depth));
    for (const c of kids.get(p.id) || []) walk(c, depth + 1);
  };
  const roots = posts.filter((p) => p.parent == null).sort((x, y) => y.id - x.id);   // threads: newest first
  roots.slice(0, shownThreads).forEach((p) => walk(p, 0));
  list.appendChild(frag);
  if (more) more.hidden = roots.length <= shownThreads;
}

function poll() {
  fetch(FEED, { cache: "no-store" })
    .then((r) => { if (!r.ok) throw new Error("http " + r.status); return r.json(); })
    .then((d) => { posts = (d && d.posts) || []; lastRead = { best: d && d.best_block }; readFailed = false; render(); })
    .catch(() => { readFailed = true; status(); /* keep what is shown; the feed will come back */ });
}

function note(kind, es, en) {
  const n = $("fo-note");
  if (!n) return;
  n.className = "post-note " + kind;
  n.textContent = t(es, en);
}

function setReply(id) {
  replyTo = id;
  const r = $("fo-replyto");
  if (!r) return;
  r.hidden = id == null;
  $("fo-replyto-id").textContent = id == null ? "" : "#" + id;
  if (id != null) $("fo-text").focus();
}

async function send() {
  const btn = $("fo-post");
  const text = ($("fo-text").value || "").trim();
  if (!text) return note("bad", "Escribe algo.", "Write something.");
  const W = await tx.openWallet();
  if (!W) return note("bad", "No hay máscara abierta en este navegador. Fórjala en el rito (/rito) y vuelve.",
                             "No mask is open in this browser. Forge one in the rite (/rito) and come back.");
  btn.disabled = true;
  try {
    note("info", "Guardando el texto con su huella…", "Storing the text under its fingerprint…");
    const hash = await tx.storeText(text);
    const parent = replyTo == null ? [0] : [1, ...u64le(replyTo)];
    note("info", "Tu máscara firma en este dispositivo…", "Your mask signs on this device…");
    const h = await tx.submitCall(W, new Uint8Array([PALLET_FORUM, CALL_POST, ...hash, ...parent]));
    note("good", "Publicado (tx " + h.slice(0, 12) + "…). Aparecerá en unos segundos.",
                 "Posted (tx " + h.slice(0, 12) + "…). It will appear in a few seconds.");
    $("fo-text").value = "";
    setReply(null);
    setTimeout(poll, 13000);
  } catch (e) {
    const m = String(e && e.message || e);
    if (m === "too long") note("bad", "El mensaje es demasiado largo (4 KiB como mucho).", "The message is too long (4 KiB at most).");
    else note("bad", "No se pudo publicar: " + m, "Could not post: " + m);
  } finally { btn.disabled = !tx.capable(); }
}

function u64le(n) {
  const b = new Uint8Array(8);
  let v = BigInt(n);
  for (let i = 0; i < 8; i++) { b[i] = Number(v & 0xffn); v >>= 8n; }
  return b;
}

document.addEventListener("click", (ev) => {
  const r = ev.target.closest("[data-reply]");
  if (r) return setReply(Number(r.dataset.reply));
  if (ev.target.closest("#fo-replyto-cancel")) return setReply(null);
  if (ev.target.closest("#forum-more")) { shownThreads += PAGE; return render(); }
});
const postBtn = $("fo-post");
if (postBtn) {
  postBtn.addEventListener("click", send);
  const arm = () => { postBtn.disabled = !tx.capable(); };
  window.addEventListener("hlqwallet-ready", arm);
  arm();
}
document.querySelectorAll(".lang button").forEach((b) => b.addEventListener("click", render));

// Arriving from a market offer (/el-foro?sobre=oferta-N): prefill the composer with the reference. Only a
// strict pattern is accepted and it goes in as plain text (.value), never markup.
(function prefillFromOffer() {
  const m = /^oferta-(\d{1,12})$/.exec(new URLSearchParams(location.search).get("sobre") || "");
  const ta = $("fo-text");
  if (!m || !ta || ta.value) return;
  ta.value = t("Sobre la oferta #" + m[1] + " del mercado: ", "About market offer #" + m[1] + ": ");
  ta.scrollIntoView({ block: "center" });
})();

poll();
setInterval(poll, POLL_MS);
