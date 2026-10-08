/* Harlequin — Circle call PREVIEW controller.  (#648 §E front-end)
 *
 * STAGED / PREVIEW — corro-call.html is linked from corro.html (call preview). This drives a *simulated* call so
 * the interaction (dial -> route lights up hop by hop -> ringing -> in-call -> hang up) can be seen and
 * reviewed before launch. There is NO network here: nothing is sent, no crypto runs. The real transport
 * (onion route via the node network) and the session crypto (forward secrecy) are the audited
 * sphinx-packet layer + the gated #637 handshake — neither lives in this file.
 *
 * External file (not inline) so the strict CSP (`style-src/script-src 'self'`) never blocks it. Every
 * DOM lookup is null-safe, so the script is inert on any page that lacks the markup.
 */
(function () {
  "use strict";
  if (typeof document === "undefined") return;

  function q(name, root) { return (root || document).querySelector('[data-call="' + name + '"]'); }
  function es() { return document.documentElement.classList.contains("lang-es"); }
  function t(esTxt, enTxt) { return es() ? esTxt : enTxt; }

  var dial = q("dial");
  var hangup = q("hangup");
  var stateEl = q("state");
  var dotEl = q("dot");
  var route = q("route");
  if (!dial || !hangup || !stateEl || !route) return; // not the preview page

  var hops = route ? route.querySelectorAll(".call-hop") : [];
  var timers = [];
  var seconds = 0;
  var tick = null;
  // Current phase drives both the status line and the language re-render.
  var phase = "idle";

  function clearTimers() {
    for (var i = 0; i < timers.length; i++) clearTimeout(timers[i]);
    timers = [];
    if (tick) { clearInterval(tick); tick = null; }
  }

  function lightHops(n) {
    for (var i = 0; i < hops.length; i++) hops[i].classList.toggle("lit", i < n);
  }

  function fmt(s) {
    var m = Math.floor(s / 60), r = s % 60;
    return m + ":" + (r < 10 ? "0" : "") + r;
  }

  function renderState() {
    var map = {
      idle: t("Listo para llamar", "Ready to call"),
      dialing: t("Tejiendo la ruta…", "Weaving the route…"),
      ringing: t("Sonando…", "Ringing…"),
      incall: t("En llamada", "In call") + " · " + fmt(seconds),
      ended: t("Llamada terminada", "Call ended")
    };
    stateEl.textContent = map[phase] || map.idle;
    if (dotEl) dotEl.className = "call-dot call-dot-" + phase;
    document.querySelector(".call-card").setAttribute("data-phase", phase);
  }

  function setPhase(p) { phase = p; renderState(); }

  function reset() {
    clearTimers();
    seconds = 0;
    lightHops(1); // "You" always lit
    setPhase("idle");
    dial.disabled = false;
    hangup.disabled = true;
  }

  function startCall() {
    clearTimers();
    dial.disabled = true;
    hangup.disabled = false;
    setPhase("dialing");
    lightHops(1);

    // Light each hop in turn (the onion route forming), then ring, then connect.
    var step = 650;
    for (var i = 2; i <= hops.length; i++) {
      (function (n) {
        timers.push(setTimeout(function () { lightHops(n); }, step * (n - 1)));
      })(i);
    }
    timers.push(setTimeout(function () { setPhase("ringing"); }, step * hops.length));
    timers.push(setTimeout(function () {
      setPhase("incall");
      seconds = 0;
      tick = setInterval(function () { seconds++; renderState(); }, 1000);
    }, step * hops.length + 1400));
  }

  function endCall() {
    clearTimers();
    hangup.disabled = true;
    setPhase("ended");
    lightHops(hops.length); // briefly hold the full route, then fade back
    timers.push(setTimeout(reset, 1600));
  }

  dial.addEventListener("click", startCall);
  hangup.addEventListener("click", endCall);
  // Keep the status line in the active language when the toggle is used mid-call.
  document.querySelectorAll(".lang button").forEach(function (b) {
    b.addEventListener("click", function () { renderState(); });
  });

  reset();
})();
