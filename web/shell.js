/* Harlequin — logged-in shell nav ("las estancias").  #680 B1
 *
 * Two worlds. Logged-OUT = the public .topbar (the concept/manifesto pages anyone can read).
 * LOGGED-IN = the estancias nav, shown only while the on-device wallet is unlocked. The Manifesto
 * stays at the dead centre (it is the constitution, always one click away); "Salir" locks the wallet.
 *
 * OPSEC: this module is pure UI. It only reads window.HlqWallet.isUnlocked() and listens for the
 * PAYLOAD-FREE 'hlqwallet-ready' / 'hlqwallet-changed' events fired by wallet-core.js. No seed, key
 * or balance ever passes through here. Loaded as an external file so a strict CSP never blocks it.
 *
 * The nav is built once and inserted after the public topbar; CSS (body.logged) decides which shows.
 * On a fresh page load we read isUnlocked() directly, so create/restore-then-navigate lands correct
 * even though those do not fire the same-page event.
 */
(function () {
  "use strict";

  // 7-oct-2026 (the maintainer: "make them feel like rooms of the villa"): the bar is the villa's own HUD, not
  // the site menu. Left: the crest and "← The Villa" (back to the square). Centre: the rooms. Right: your mask
  // (a link to your dressing room, /panel) and "Leave". Standing lives inside My mask (/panel links it);
  // the Manifesto stays one click away at the end of the rooms.
  var HOME = { es: "La Villa", en: "The Villa", href: "/villa/entrar/" };
  var ROOMS = [
    { es: "Cartera", en: "Wallet",   href: "/wallet"  },  // el Banco de la villa — #664
    { es: "Mercado", en: "Market",   href: "/market"  },  // NOT /plaza (/plaza = concept page)
    { es: "Foro",    en: "Forum",    href: "/el-foro" },  // foro/debate #651
    { es: "Corro",   en: "Circle",   href: "/corro"   },
    { es: "Consejo", en: "Council",  href: "/consejo" }
  ];
  var MANIFESTO = { es: "Manifiesto", en: "Manifesto", href: "/" };
  var MASK = { es: "Mi máscara", en: "My mask", href: "/panel" };
  var LEAVE = { es: "Salir", en: "Leave", leave: true };   // = HlqWallet.lock(), then back to the villa door

  // Normalise a path for active-state matching: strip trailing slash and .html (Caddy serves
  // extensionless routes; the local dev server serves .html). "" -> "/".
  function norm(p) {
    p = (p || "").replace(/\/+$/, "").replace(/\.html$/, "");
    return p === "" ? "/" : p;
  }

  function label(item, el) {
    var es = document.createElement("span"); es.className = "es"; es.lang = "es"; es.textContent = item.es;
    var en = document.createElement("span"); en.className = "en"; en.lang = "en"; en.textContent = item.en;
    el.appendChild(es); el.appendChild(en);
    return el;
  }

  // Cross-page shell visibility. The wallet key lives in memory and is released on navigation, so
  // HlqWallet.isUnlocked() is false on every fresh page — it cannot, alone, keep the shell across
  // estancias. We mirror "the shell is open" into a sessionStorage flag (set on unlock, cleared on
  // lock/Salir). This is UI STATE ONLY, never a security boundary: it just chooses which nav chrome
  // shows. Every sensitive action still requires the in-memory unlock (and the server session for
  // server calls); a forged flag reveals no secret and unlocks nothing. Cleared when the tab closes.
  var FLAG = "hlq-shell";
  function flagged() { try { return sessionStorage.getItem(FLAG) === "1"; } catch (e) { return false; } }
  function setFlag(on) { try { on ? sessionStorage.setItem(FLAG, "1") : sessionStorage.removeItem(FLAG); } catch (e) {} }

  function item(it) {
    var here = norm(location.pathname), el;
    if (it.leave) {
      el = document.createElement("button");
      el.type = "button"; el.className = "leave";
      el.addEventListener("click", function () {
        setFlag(false);                                   // works on every page, wallet present or not
        try { sessionStorage.removeItem("hlq.villa.session"); } catch (e) {}  // the villa's own login marker
        if (window.HlqWallet) window.HlqWallet.lock();     // releases the key where it is loaded
        sync();
        location.href = HOME.href;                         // leaving a room of the villa = back to its door
      });
    } else if (it.inert) {
      el = document.createElement("span");
      el.className = "inert"; el.setAttribute("aria-disabled", "true");
    } else {
      el = document.createElement("a");
      el.href = it.href;
      if (norm(it.href) === here) { el.className = "active"; el.setAttribute("aria-current", "page"); }
    }
    return label(it, el);
  }

  // The handle shown on the mask chip: public (it IS the address), never a secret. From the villa's own
  // login marker, or from the wallet when it is open on this page. Unknown → the chip just says «Mi máscara».
  function handle() {
    try {
      var v = JSON.parse(sessionStorage.getItem("hlq.villa.session") || "null");
      if (v && typeof v.h === "string" && /^hlq-[a-z2-7]{10,40}$/.test(v.h)) return v.h;
    } catch (e) {}
    var a = window.HlqWallet && window.HlqWallet.address && window.HlqWallet.address();
    return (typeof a === "string" && /^hlq-[a-z2-7]{10,40}$/.test(a)) ? a : "";
  }

  function paintMask() {
    var chip = document.querySelector(".topbar.shell .vb-mask");
    if (!chip) return;
    var h = handle(), out = chip.querySelector(".vb-handle"), lab = chip.querySelector(".vb-label");
    out.textContent = h; out.hidden = !h; lab.hidden = !!h;
    chip.title = h ? h : "";
  }

  function build() {
    if (document.querySelector(".topbar.shell")) return;
    var pub = document.querySelector(".topbar");
    if (!pub || !pub.parentNode) return;
    var nav = document.createElement("nav");
    nav.className = "topbar shell";
    nav.setAttribute("aria-label", "La villa / The villa");

    var left = document.createElement("div"); left.className = "topbar-left";
    var home = document.createElement("a"); home.href = HOME.href; home.className = "vb-home";
    var img = document.createElement("img"); img.src = "/logo-gold.png"; img.alt = ""; img.width = 28; img.height = 28;
    var back = document.createElement("span"); back.className = "vb-back"; back.setAttribute("aria-hidden", "true"); back.textContent = "←";
    home.appendChild(img); home.appendChild(back); label(HOME, home);
    left.appendChild(home);

    var center = document.createElement("div"); center.className = "topbar-center vb-rooms";
    ROOMS.forEach(function (it) { center.appendChild(item(it)); });
    var dot = document.createElement("span"); dot.className = "sep"; dot.setAttribute("aria-hidden", "true"); dot.textContent = "·";
    center.appendChild(dot);
    var man = item(MANIFESTO); man.classList.add("vb-manifesto"); center.appendChild(man);

    var right = document.createElement("div"); right.className = "topbar-right";
    var chip = item(MASK); chip.classList.add("vb-mask");
    var lab = document.createElement("span"); lab.className = "vb-label";
    while (chip.firstChild) lab.appendChild(chip.firstChild);
    var led = document.createElement("span"); led.className = "vb-dot"; led.setAttribute("aria-hidden", "true");
    var h = document.createElement("span"); h.className = "vb-handle"; h.hidden = true;
    chip.appendChild(led); chip.appendChild(lab); chip.appendChild(h);
    right.appendChild(chip);
    var leave = item(LEAVE); right.appendChild(leave);

    nav.appendChild(left); nav.appendChild(center); nav.appendChild(right);
    pub.parentNode.insertBefore(nav, pub.nextSibling);
    paintMask();
  }

  function sync() {
    var on = !!(window.HlqWallet && window.HlqWallet.isUnlocked()) || flagged();
    document.body.classList.toggle("logged", on);
  }

  // On a real unlock/lock, reconcile the cross-page flag with the live wallet state, then re-sync.
  function onChange() {
    setFlag(!!(window.HlqWallet && window.HlqWallet.isUnlocked()));
    sync(); paintMask();
  }

  function init() { build(); sync(); }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
  // 1-oct-2026: "ready" is NOT a lock. The wallet loads LOCKED on every page; treating its arrival as a lock
  // cleared the cross-page flag and threw you out of the villa on /wallet. On ready, only ever confirm the flag.
  window.addEventListener("hlqwallet-ready", function () {
    if (window.HlqWallet && window.HlqWallet.isUnlocked()) setFlag(true);
    sync(); paintMask();
  });
  window.addEventListener("hlqwallet-changed", onChange); // unlock / lock, same page
})();
