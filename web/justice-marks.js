/* Harlequin — the public mark of a guilty verdict, next to a name (2026-09-24).  [ES MODULE]
 *
 * Moderation-B (the maintainer, 2026-07-15): nothing is ever hidden. When a sortition jury finds a mask
 * guilty, the chain keeps the fact (pallet-justice GuiltyMarks) and every reader sees it next to that
 * mask's words and offers — and decides for themselves (Art. IX: "known truth, and freedom to act on it").
 * No mark → nothing is shown: silence is not a certificate of innocence, just the absence of a verdict.
 */
const ENDPOINT = "/api/justice/marks";
const cache = new Map();   // pubkey hex → Promise<number[]>

async function load(keys) {
  const want = [...new Set(keys)].filter((k) => !cache.has(k));
  for (let i = 0; i < want.length; i += 64) {
    const chunk = want.slice(i, i + 64);
    const p = fetch(ENDPOINT + "?accounts=" + chunk.join(","), { cache: "no-store" })
      .then((r) => (r.ok ? r.json() : { marks: {} }))
      .then((d) => (d && d.marks) || {})
      .catch(() => ({}));
    chunk.forEach((k) => cache.set(k, p.then((m) => m[k] || [])));
  }
  return Promise.all(keys.map((k) => cache.get(k)));
}

/** Append a verdict label to `el` for `pubkey` if the chain holds any guilty verdict against it. */
export function markInto(el, pubkey, lang) {
  load([pubkey]).then(([cases]) => {
    if (!cases || !cases.length) return;
    const s = document.createElement("span");
    s.className = "verdict-mark";
    const n = cases.length;
    s.textContent = lang() === "es"
      ? "⚖ " + n + (n === 1 ? " veredicto de culpa" : " veredictos de culpa")
      : "⚖ " + n + (n === 1 ? " guilty verdict" : " guilty verdicts");
    s.title = (lang() === "es" ? "Casos: #" : "Cases: #") + cases.join(", #");
    el.appendChild(s);
  });
}

/** Warm the cache for many authors at once (one request per 64). */
export function prefetch(pubkeys) { load(pubkeys); }
