(function () {
  var KEY = "hlq-lang";
  var valid = { es: 1, en: 1 };

  // Route slugs per language: [es, en, ...legacy aliases]. Every alias resolves
  // server-side on any host; here we localize hrefs so each domain shows its own
  // language's paths (proyectoharlequin.org → ES slugs, harlequinproject.org → EN).
  var ROUTES = [
    ["como-funciona", "how-it-works"],
    ["tesis", "thesis"],
    ["red", "network"],
    ["diario", "journal", "blog"],
    ["unete", "join"],
    ["registro", "register"],
    ["por-dentro", "under-the-hood"]
  ];
  var SLUG_INDEX = {};
  ROUTES.forEach(function (pair) {
    pair.forEach(function (slug) { SLUG_INDEX["/" + slug] = pair; });
  });

  function localizeLinks(lang) {
    var idx = lang === "es" ? 0 : 1;
    document.querySelectorAll('a[href^="/"]').forEach(function (a) {
      var href = a.getAttribute("href");
      var m = href.match(/^(\/[a-z-]+)\/?([?#].*)?$/);
      if (!m) return; // deeper paths (/blog/post) and assets stay untouched
      var pair = SLUG_INDEX[m[1]];
      if (pair) a.setAttribute("href", "/" + pair[idx] + (m[2] || ""));
    });
  }

  function apply(lang) {
    if (!valid[lang]) lang = "en";
    // Class on <html> (not <body>) so the inline head script can pre-set it before paint (no flash).
    document.documentElement.classList.toggle("lang-es", lang === "es");
    var root = document.documentElement;
    root.lang = lang;
    // Browser tab + meta description follow the active language. Static HTML stays
    // English for link-preview crawlers (they do not run JS); these data-* values
    // live on <html> so the script is page-agnostic (manifesto + roadmap).
    var title = root.getAttribute("data-title-" + lang);
    if (title) document.title = title;
    var desc = root.getAttribute("data-desc-" + lang);
    var descMeta = document.querySelector('meta[name="description"]');
    if (desc && descMeta) descMeta.setAttribute("content", desc);
    var btns = document.querySelectorAll(".lang button");
    for (var i = 0; i < btns.length; i++) {
      btns[i].classList.toggle("active", btns[i].getAttribute("data-lang") === lang);
    }
    localizeLinks(lang);
    try { localStorage.setItem(KEY, lang); } catch (e) {}
  }

  var initial;
  try { initial = localStorage.getItem(KEY); } catch (e) {}
  if (!valid[initial]) {
    // No saved choice: default by domain. The Spanish site lives on proyectoharlequin.org;
    // every other host (harlequinproject.org) defaults to English. The toggle still works,
    // and localStorage is per-origin so a choice on one domain never leaks to the other.
    initial = /(^|\.)proyectoharlequin\.org$/i.test(location.hostname) ? "es" : "en";
  }

  apply(initial);

  document.querySelectorAll(".lang button").forEach(function (b) {
    b.addEventListener("click", function () { apply(b.getAttribute("data-lang")); });
  });
})();

/* Copy button on every command box (the maintainer, 2026-08-01: "the command boxes need that thing
   you click and it copies itself"). Built here, not in the pages, so a new `.cmd` anywhere gets it
   for free. No inline handlers and no inline script: the site runs under a strict CSP that would
   block them. */
(function () {
  "use strict";
  function label(es, en) {
    return document.documentElement.classList.contains("lang-es") ? es : en;
  }
  function copy(text) {
    if (navigator.clipboard && window.isSecureContext) return navigator.clipboard.writeText(text);
    // Fallback for a non-secure origin (local review server): a throwaway textarea + execCommand.
    return new Promise(function (resolve, reject) {
      var ta = document.createElement("textarea");
      ta.value = text; ta.setAttribute("readonly", "");
      ta.style.position = "fixed"; ta.style.opacity = "0";
      document.body.appendChild(ta); ta.select();
      var ok = false;
      try { ok = document.execCommand("copy"); } catch (e) { ok = false; }
      document.body.removeChild(ta);
      ok ? resolve() : reject();
    });
  }
  document.querySelectorAll(".cmd").forEach(function (box) {
    if (box.querySelector(".cmd-copy")) return;
    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "cmd-copy";
    btn.textContent = label("copiar", "copy");
    // The command text only: the button's own label must never end up in the clipboard.
    btn.addEventListener("click", function () {
      var text = "";
      box.childNodes.forEach(function (n) { if (n !== btn) text += n.textContent; });
      copy(text.trim()).then(function () {
        btn.textContent = label("copiado", "copied");
        btn.classList.add("done");
        setTimeout(function () {
          btn.textContent = label("copiar", "copy");
          btn.classList.remove("done");
        }, 1600);
      }).catch(function () {
        btn.textContent = label("copia a mano", "copy by hand");
      });
    });
    box.appendChild(btn);
  });
})();
