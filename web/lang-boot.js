/* Set the language class before paint so the Spanish site never flashes English first.
 * External (not inline) so the strict CSP (script-src 'self', no unsafe-inline) allows it; an inline
 * <script> was silently blocked site-wide, killing the pre-paint set. Loaded synchronously in <head>. */
(function () {
  try {
    var host = location.hostname, k;
    // The public domain is AUTHORITATIVE: the EN domain always opens in English, the ES domain in Spanish
    // (a stored toggle must never make the EN link render Spanish). localStorage only decides on domains
    // with no language of their own (the .onion / localhost), where the toggle is the only signal.
    if (/(^|\.)proyectoharlequin\.org$/i.test(host)) k = "es";
    else if (/(^|\.)harlequinproject\.org$/i.test(host)) k = "en";
    else { k = localStorage.getItem("hlq-lang"); if (k !== "es" && k !== "en") k = "en"; }
    if (k === "es") document.documentElement.classList.add("lang-es");
    // Pages that do not load app.js (the rite) still need their tab title in the right language.
    var root = document.documentElement; root.lang = k;
    document.addEventListener("DOMContentLoaded", function () {
      var t = root.getAttribute("data-title-" + k), d = root.getAttribute("data-desc-" + k);
      if (t) document.title = t;
      var m = document.querySelector('meta[name="description"]'); if (d && m) m.setAttribute("content", d);
    });
  } catch (e) {}
})();
