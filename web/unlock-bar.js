/* Harlequin — "open your mask" (2026-09-23). Shared by /panel and /market.
 *
 * Since 2026-09-23 the rite OFFERS a passphrase (optional, recommended). A mask without one opens by
 * itself (panel.js / market.js try the empty passphrase); a mask WITH one used to leave those pages as a
 * static preview with no way in. This bar appears only when there is a sealed mask on this device, it is
 * still locked, and the empty passphrase did not open it. The passphrase goes straight into the on-device
 * wallet (wallet-core.js → wasm) and is never sent anywhere. CSP-clean: external file, no inline handlers.
 */
(function () {
  "use strict";
  var $ = function (id) { return document.getElementById(id); };
  function lang() { return document.documentElement.classList.contains("lang-es") ? "es" : "en"; }
  function t(es, en) { return lang() === "es" ? es : en; }

  function build() {
    if ($("unlock-bar")) return $("unlock-bar");
    var bar = document.createElement("form");
    bar.id = "unlock-bar";
    bar.className = "unlock-bar";
    bar.setAttribute("autocomplete", "off");
    var label = document.createElement("label");
    label.setAttribute("for", "unlock-bar-pass");
    label.textContent = t("Tu máscara tiene cerrojo. Escribe tu frase de paso para abrirla aquí:",
                          "Your mask is locked. Type your passphrase to open it here:");
    var input = document.createElement("input");
    input.type = "password"; input.id = "unlock-bar-pass"; input.autocomplete = "current-password";
    var btn = document.createElement("button");
    btn.type = "submit"; btn.textContent = t("Abrir", "Open");
    var msg = document.createElement("span");
    msg.className = "unlock-bar-msg"; msg.id = "unlock-bar-msg";
    bar.appendChild(label); bar.appendChild(input); bar.appendChild(btn); bar.appendChild(msg);
    bar.addEventListener("submit", function (ev) {
      ev.preventDefault();
      var W = window.HlqWallet;
      if (!W) return;
      btn.disabled = true;
      W.unlock(input.value).then(function () {
        input.value = "";
        bar.remove();                       // hlqwallet-changed fires → the page renders the owner view
      }).catch(function () {
        msg.textContent = t("Frase de paso incorrecta.", "Wrong passphrase.");
      }).then(function () { btn.disabled = false; });
    });
    // Under the page's «preview» banner when there is one (the panel): inserting at the top of <main>
    // dropped the bar INTO the card grid, where it read as one more card.
    var banner = document.querySelector(".preview-banner");
    if (banner && banner.parentNode) banner.parentNode.insertBefore(bar, banner.nextSibling);
    else { var main = document.querySelector("main") || document.body; main.insertBefore(bar, main.firstChild); }
    return bar;
  }

  function check() {
    var W = window.HlqWallet;
    if (!W || W.isUnlocked()) return;
    W.hasKeystore().then(function (has) {
      if (!has) return;
      // same order as the pages: the empty passphrase first (the rite's unlocked mask)
      return W.unlock("").catch(function () { if (!W.isUnlocked()) build(); });
    });
  }

  window.addEventListener("hlqwallet-ready", check);
  if (window.HlqWallet) check();
})();
