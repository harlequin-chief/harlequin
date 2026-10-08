/* Station 5 — the screen. All user-facing wording lives here; `bind.js` holds no strings and this
 * file holds no crypto. Two rules shape everything below:
 *
 *   1. NOBODY SIGNS BLIND. The first press does not sign: it shows, in plain words, what is about to
 *      be declared and — when it applies — that this REPLACES the node already claimed. The second
 *      press signs. A "Sign" button over a wall of bytes is exactly what we criticise elsewhere.
 *   2. NEVER LEAVE SOMEONE UNSURE. Any failure says whether the claim reached the chain or not,
 *      because "I don't know if my node is claimed" is the worst place to leave a person.
 */
(function () {
  "use strict";

  var $ = function (id) { return document.getElementById(id); };
  function es() { return document.documentElement.classList.contains("lang-es"); }
  function t(spanish, english) { return es() ? spanish : english; }
  function shortHex(h) { return h ? h.slice(0, 10) + "…" + h.slice(-6) : ""; }

  var pending = null;   // the reviewed claim, waiting for the second press
  var busy = false;

  function say(msg, kind) {
    var out = $("bind-out");
    if (!out) return;
    out.textContent = msg;
    out.className = "bind-out" + (kind ? " " + kind : "");
  }

  function setButton(labelEs, labelEn) {
    var b = $("bind-go");
    if (!b) return;
    b.textContent = "";
    var a = document.createElement("span"); a.className = "es"; a.lang = "es"; a.textContent = labelEs;
    var c = document.createElement("span"); c.className = "en"; c.lang = "en"; c.textContent = labelEn;
    b.appendChild(a); b.appendChild(c);
  }

  /** The review panel: what you are about to declare, in words, before anything is signed. */
  function showReview(detail) {
    var box = $("bind-review");
    box.textContent = "";
    box.hidden = false;

    var lines = [];
    lines.push(t(
      "Vas a declarar que el nodo con esta llave trabaja para tu máscara:",
      "You are about to declare that the node with this key works for your mask:"));
    lines.push("· " + t("tu máscara", "your mask") + ": " + shortHex(detail.account));
    lines.push("· " + t("el nodo", "the node") + ": " + shortHex(detail.voteKey));
    lines.push(t(
      "La firma se hace aquí, en tu equipo, y caduca en unos minutos si no se envía.",
      "The signature happens here, on your device, and expires in minutes if not sent."));

    lines.forEach(function (line) {
      var p = document.createElement("p");
      p.className = "bind-line";
      p.textContent = line;
      box.appendChild(p);
    });

    // Criterion: what is shown must be COMPARABLE with what is signed, not merely described. Folded
    // away so it does not shout at someone who does not care, and there in full for someone who does
    // — and the engine refuses to sign anything but exactly these bytes.
    if (detail.payloadHex) {
      var det = document.createElement("details");
      det.className = "bind-bytes";
      var sum = document.createElement("summary");
      sum.textContent = t("Ver los bytes exactos que se van a firmar",
                          "Show the exact bytes about to be signed");
      var pre = document.createElement("p");
      pre.className = "bind-hex";
      pre.textContent = detail.payloadHex;
      var note = document.createElement("p");
      note.className = "bind-line";
      note.textContent = t(
        "Si estos bytes cambian antes de que firmes, la firma se cancela sola y tendrás que revisar otra vez.",
        "If these bytes change before you sign, the signature is cancelled and you will have to review again.");
      det.appendChild(sum); det.appendChild(pre); det.appendChild(note);
      box.appendChild(det);
    }

    if (detail.replaces) {
      // The only path that can hurt someone: two values from a stranger, pasted in good faith.
      var warn = document.createElement("p");
      warn.className = "bind-warn";
      warn.textContent = t(
        "ATENCIÓN: tu máscara ya tiene un nodo declarado (" + shortHex(detail.replaces) +
          "). Si firmas, ese deja de contar para ti y pasa a contar este. Si no reconoces esa llave, PARA.",
        "CAREFUL: your mask already has a node declared (" + shortHex(detail.replaces) +
          "). If you sign, that one stops counting for you and this one takes its place. If you do not recognise that key, STOP.");
      box.appendChild(warn);
    }
  }

  function reset(msgEs, msgEn, kind) {
    pending = null;
    busy = false;
    $("bind-review").hidden = true;
    setButton("Revisar lo que voy a firmar", "Review what I am about to sign");
    if (msgEs) say(t(msgEs, msgEn), kind);
  }

  /* First press: read the chain and show the review. Nothing is signed here. */
  function review() {
    var key = ($("bind-key").value || "").trim();
    var proof = ($("bind-proof").value || "").trim();
    var rpc = ($("bind-rpc").value || "").trim();
    if (!key || !proof) {
      say(t("Faltan los dos valores que imprime tu nodo.", "Both values printed by your node are missing."), "bad");
      return;
    }
    if (!window.HlqWallet || !window.HlqWallet.isUnlocked || !window.HlqWallet.isUnlocked()) {
      say(t("Desbloquea tu máscara primero: la firma la haces tú, aquí.",
            "Unlock your mask first: you are the one signing, here."), "bad");
      return;
    }
    busy = true;
    say(t("Preguntando a la cadena…", "Asking the chain…"));

    var accountHex = window.HlqWallet.pubkeyHex();
    Promise.all([
      window.HlqBind.currentBinding(rpc, window.HlqBind._internals.hexToBytes(accountHex)),
      window.HlqBind.previewPayload(rpc, accountHex, key, proof)
    ])
      .then(function (r) {
        var already = r[0], preview = r[1];
        pending = { key: key, proof: proof, rpc: rpc, replaces: already, payloadHex: preview.payloadHex };
        showReview({ account: accountHex, voteKey: key, replaces: already, payloadHex: preview.payloadHex });
        busy = false;
        say("");
        if (already) {
          setButton("Firmar y SUSTITUIR el nodo anterior", "Sign and REPLACE the previous node");
        } else {
          setButton("Firmar con mi máscara y enviar", "Sign with my mask and send");
        }
      })
      .catch(function (e) {
        busy = false;
        say(t("No se pudo preguntar a la cadena (" + e.message + "). No se ha firmado ni enviado nada.",
              "Could not ask the chain (" + e.message + "). Nothing was signed or sent."), "bad");
      });
  }

  /* Second press: sign and submit. Every outcome states whether the claim reached the chain. */
  function confirm() {
    if (!pending || busy) return;
    busy = true;
    say(t("Firmando en tu equipo…", "Signing on your device…"));

    // From the moment the send is attempted, the outcome is UNKNOWN until the chain says otherwise.
    // Reporting "nothing was sent" after a dropped connection would be a lie that reassures — the
    // worst kind — because the claim may well have landed and replaced a previous binding.
    var attempted = false;
    var claim = pending;

    window.HlqBind.claimNode({
      rpcUrl: claim.rpc,
      votePubHex: claim.key,
      proofHex: claim.proof,
      confirmReplace: true,             // the human already saw the replacement warning above
      expectReplaces: claim.replaces,   // …for THAT key: if it changed meanwhile, refuse and re-ask
      expectPayloadHex: claim.payloadHex, // …and for THOSE bytes: shown == signed, or nothing happens
      onStep: function (state) {
        if (state === "submitting") {
          attempted = true;
          say(t("Enviando a la cadena…", "Sending to the chain…"));
        }
      }
    })
      .then(function (hash) {
        reset();
        say(t("Hecho. Tu nodo queda declarado como tuyo (operación " + shortHex(hash) +
                "). Tarda unos segundos en entrar en un bloque.",
              "Done. Your node is declared as yours (transaction " + shortHex(hash) +
                "). It takes a few seconds to enter a block."), "good");
      })
      .catch(function (e) {
        var refused = e && e.message && /rpc:/.test(e.message);   // the chain answered, with a no
        reset();

        if (e && e.code === "would-replace") {
          say(t("Tu máscara ya tiene otro nodo declarado. Vuelve a revisar antes de sustituirlo.",
                "Your mask already has another node declared. Review again before replacing it."), "bad");
          return;
        }
        if (e && e.code === "payload-changed") {
          say(t("Los bytes cambiaron entre la revisión y la firma (la cadena avanzó): no se firmó nada. Revisa otra vez.",
                "The bytes changed between review and signing (the chain moved on): nothing was signed. Review again."), "bad");
          return;
        }
        if (e && e.code === "changed-since-review") {
          say(t("Lo que te mostramos ha cambiado mientras decidías: no se ha firmado nada. Revisa otra vez.",
                "What we showed you changed while you decided: nothing was signed. Review again."), "bad");
          return;
        }
        if (refused) {
          say(t("La cadena rechazó la declaración: " + e.message +
                  ". Tu nodo NO ha quedado declarado; nada ha cambiado.",
                "The chain refused the claim: " + e.message +
                  ". Your node is NOT declared; nothing changed."), "bad");
          return;
        }
        if (!attempted) {
          say(t("No se completó (" + e.message + "). No se envió nada: tu nodo sigue como estaba.",
                "It did not complete (" + e.message + "). Nothing was sent: your node is as it was."), "bad");
          return;
        }
        // Sent, and then the answer never arrived. Do not guess — go and look, and say what is there.
        say(t("Se envió, pero se perdió la respuesta (" + e.message +
                "). NO sabemos si entró. Comprobando en la cadena…",
              "It was sent, but the answer was lost (" + e.message +
                "). We do NOT know whether it landed. Checking the chain…"), "bad");
        window.HlqBind.currentBinding(claim.rpc,
          window.HlqBind._internals.hexToBytes(window.HlqWallet.pubkeyHex()))
          .then(function (now) {
            var claimed = now && now.toLowerCase() === claim.key.toLowerCase();
            if (claimed) {
              say(t("Sí entró: tu nodo QUEDÓ declarado. No lo repitas.",
                    "It did land: your node IS declared. Do not repeat it."), "good");
            } else if (now) {
              say(t("No entró esa declaración. Tu máscara sigue con el nodo " + shortHex(now) + ".",
                    "That claim did not land. Your mask still has node " + shortHex(now) + "."), "bad");
            } else {
              say(t("No entró: tu máscara sigue sin nodo declarado. Puedes intentarlo otra vez.",
                    "It did not land: your mask still has no node declared. You can try again."), "bad");
            }
          })
          .catch(function () {
            say(t("Se envió y no hemos podido comprobar el resultado. Mira tu nodo antes de repetirlo: " +
                    "repetir sobre una declaración que sí entró no rompe nada, pero conviene saberlo.",
                  "It was sent and we could not check the result. Look at your node before repeating: " +
                    "repeating over a claim that did land breaks nothing, but you should know."), "bad");
          });
      });
  }

  function init() {
    var b = $("bind-go");
    if (!b || !window.HlqBind) return;
    b.addEventListener("click", function () {
      if (pending) confirm(); else review();
    });
    // Editing any field invalidates a review: what you sign must be what you are looking at.
    ["bind-key", "bind-proof", "bind-rpc"].forEach(function (id) {
      var el = $(id);
      if (el) el.addEventListener("input", function () { if (pending) reset(); });
    });
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
})();
