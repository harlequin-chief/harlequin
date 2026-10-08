    import init, * as W from "./wallet/wallet_wasm.js";

    const $ = (id) => document.getElementById(id);
    const KS_KEY = "hlq.keystore.v1";
    // Bridge 2026-08-03 (four-eyes with Forge): the RITE stores the sealed keystore in indexedDB
    // ("hlq-wallet"/keystore/default, via wallet-core.js) and this door only looked at localStorage —
    // a freshly forged mask "did not exist" here. Same blob (sealKeystore from the same wasm), only
    // the drawer changes: if localStorage is empty the rite's one is read, and once opened
    // successfully it migrates to localStorage so the rest of the flow stays identical.
    let RITO_BLOB = null;
    function ritoBlob() {
      return new Promise((res) => {
        try {
          const r = indexedDB.open("hlq-wallet", 2);   // same version as wallet-core.js (2026-09-25)
          r.onupgradeneeded = () => { if (!r.result.objectStoreNames.contains("keystore")) r.result.createObjectStore("keystore"); };
          r.onerror = () => res(null);
          r.onsuccess = () => {
            try {
              const db = r.result;
              if (!db.objectStoreNames.contains("keystore")) { db.close(); return res(null); }
              const g = db.transaction("keystore", "readonly").objectStore("keystore").get("default");
              g.onsuccess = () => { db.close(); res(g.result ? new Uint8Array(g.result) : null); };
              g.onerror = () => { db.close(); res(null); };
            } catch (e) { res(null); }
          };
        } catch (e) { res(null); }
      });
    }
    // 2026-09-25: create/recover here used to write ONLY to localStorage, and the dressing room (wallet-core)
    // reads ONLY from indexedDB → "the dressing room does not see my mask". Now it is saved in BOTH drawers;
    // the indexedDB one is canonical (the one the rite, the dressing room and the market read).
    function idbPutBlob(u8) {
      return new Promise((res, rej) => {
        const r = indexedDB.open("hlq-wallet", 2);
        r.onupgradeneeded = () => { if (!r.result.objectStoreNames.contains("keystore")) r.result.createObjectStore("keystore"); };
        r.onerror = () => rej(r.error);
        r.onsuccess = () => {
          try {
            const db = r.result, t = db.transaction("keystore", "readwrite");
            t.objectStore("keystore").put(u8, "default");
            t.oncomplete = () => { db.close(); res(); };
            t.onerror = () => { db.close(); rej(t.error); };
          } catch (e) { rej(e); }
        };
      });
    }
    const sameBytes = (a, b) => !!a && !!b && a.length === b.length && a.every((x, i) => x === b[i]);
    // Replacing ANOTHER already-stored mask leaves it recoverable only with its 24 words: it asks first.
    function okToReplace(newBlob) {
      if (!RITO_BLOB || sameBytes(RITO_BLOB, newBlob)) return true;
      return window.confirm("Another mask is already stored in this browser. If you go on it is replaced, and only its " +
        "24 words can bring it back. Continue?\n\nYa hay otra máscara guardada en este navegador. Si sigues, se " +
        "sustituye y solo podrás recuperarla con sus 24 palabras. ¿Seguir?");
    }
    async function storeBoth(u8) {
      localStorage.setItem(KS_KEY, hex(u8));
      await idbPutBlob(u8); RITO_BLOB = u8;
    }
    let wallet = null;          // live Wallet (unlocked); dropped on lock/leave
    let sessionExp = null;

    // #973 (2026-09-23): "on return, the villa recognises you". A signed login used to live only in this
    // page's memory, so a reload threw you back to the door. Same rule as shell.js (#680): a UI-ONLY
    // marker in sessionStorage — the handle and a 30-min expiry, nothing secret. A forged marker shows a
    // name and unlocks nothing: every act that needs the key still needs the mask open (see bind below).
    // Dies with the tab, and on "Leave" / "Close mask".
    const SESSION_KEY = "hlq.villa.session";
    const SESSION_MS = 30 * 60 * 1000;
    function rememberSession(handle) {
      try { sessionStorage.setItem(SESSION_KEY, JSON.stringify({ h: handle, exp: Date.now() + SESSION_MS })); sessionStorage.setItem("hlq-shell", "1"); } catch (e) {}
      document.body.classList.add("logged");  // swap the public nav for the estancias nav (shell.js)
    }
    function forgetSession() { try { sessionStorage.removeItem(SESSION_KEY); sessionStorage.removeItem("hlq-shell"); } catch (e) {} document.body.classList.remove("logged"); }
    function rememberedSession() {
      try {
        const s = JSON.parse(sessionStorage.getItem(SESSION_KEY) || "null");
        if (s && typeof s.h === "string" && /^hlq-[a-z2-7]{10,40}$/.test(s.h) && s.exp > Date.now()) return s;
      } catch (e) {}
      forgetSession();
      return null;
    }

    const show = (screen) => {
      document.querySelectorAll(".v-screen").forEach(s => s.classList.remove("active"));
      $(screen).classList.add("active");
      // 27-sep-2026: once inside, the title "Enter with your mask" was redundant (the maintainer's walkthrough as a newcomer).
      document.body.classList.toggle("in-villa", screen === "s-shell");
      if (screen === "s-shell") startWorld(); else closePlace();
    };
    const status = (id, cls, es, en) => {
      const el = $(id);
      el.className = "v-status " + cls;
      el.innerHTML = `<span class="es" lang="es">${es}</span><span class="en" lang="en">${en}</span>`;
    };
    const hex = (u8) => [...u8].map(b => b.toString(16).padStart(2, "0")).join("");
    const unhex = (s) => {
      if (typeof s !== "string" || !s) return null; // missing field → clean null, never a cryptic .trim (four-eyes 2026-08-03)
      s = s.trim().replace(/^0x/i, "").replace(/\s+/g, "");
      if (!/^[0-9a-fA-F]*$/.test(s) || s.length % 2) return null;
      return new Uint8Array(s.match(/../g)?.map(h => parseInt(h, 16)) ?? []);
    };
    const rand = (n) => crypto.getRandomValues(new Uint8Array(n));
    const enc = new TextEncoder();

    // ── gate: tabs ──
    document.querySelectorAll(".v-tab").forEach(t => t.addEventListener("click", () => {
      document.querySelectorAll(".v-tab").forEach(x => x.classList.remove("active"));
      t.classList.add("active");
      ["unlock","create","import"].forEach(k => $("tab-"+k).classList.toggle("v-hide", t.dataset.tab !== k));
    }));

    // ── gate: unlock ──
    function refreshGate() {
      const has = !!localStorage.getItem(KS_KEY) || !!RITO_BLOB;
      $("unlock-none").classList.toggle("v-hide", has);
      $("btn-unlock").disabled = !has;
    }
    $("btn-unlock").addEventListener("click", async () => {
      try {
        const ls = localStorage.getItem(KS_KEY);
        const blob = RITO_BLOB || (ls ? unhex(ls) : null)   /* indexedDB (canonical) first — four-eyes Forge 25-sep */;
        const entropy = W.openKeystore(blob, enc.encode($("unlock-pass").value));
        if (!ls && RITO_BLOB) localStorage.setItem(KS_KEY, hex(RITO_BLOB)); // migrates the rite's drawer
        // and the other way round (2026-09-25): a mask created here before the fix used to live ONLY in
        // localStorage → it is copied to indexedDB so the dressing room can see it. Only if there is NONE
        // there yet (never overwrites another).
        if (ls && !RITO_BLOB) { try { await idbPutBlob(blob); RITO_BLOB = blob; } catch (e) {} }
        wallet = W.Wallet.fromEntropy(new Uint8Array(entropy));
        entropy.fill?.(0);
        $("unlock-pass").value = "";
        enterLoginScreen();
      } catch {
        status("gate-status", "err", "Frase de paso incorrecta.", "Wrong passphrase.");
      }
    });

    // ── gate: create ──
    let pendingEntropy = null;
    $("btn-gen").addEventListener("click", () => {
      pendingEntropy = rand(32);
      const words = W.entropyToPhrase(pendingEntropy).split(" ");
      $("phrase-grid").innerHTML = words.map((w, i) => `<span><b>${i+1}</b>${w}</span>`).join("");
      $("gen-out").classList.remove("v-hide");
    });
    $("create-ack").addEventListener("change", e => $("btn-save-new").disabled = !e.target.checked);
    $("btn-save-new").addEventListener("click", async () => {
      const pass = $("create-pass").value;
      if (!pass || pass.length < 8) {
        return status("gate-status", "err", "La frase de paso necesita al menos 8 caracteres.", "Passphrase needs at least 8 characters.");
      }
      const blob = new Uint8Array(W.sealKeystore(pendingEntropy, enc.encode(pass), rand(16), rand(24)));
      if (!okToReplace(blob)) return;
      try { await storeBoth(blob); } catch (e) {
        return status("gate-status", "err", "No se pudo guardar la máscara en este navegador.", "Could not store the mask in this browser.");
      }
      wallet = W.Wallet.fromEntropy(pendingEntropy);
      pendingEntropy.fill(0); pendingEntropy = null;
      $("create-pass").value = ""; $("phrase-grid").innerHTML = ""; $("gen-out").classList.add("v-hide");
      enterLoginScreen();
    });

    // ── gate: import ──
    $("btn-import").addEventListener("click", async () => {
      try {
        const pass = $("import-pass").value;
        if (!pass || pass.length < 8) {
          return status("gate-status", "err", "La frase de paso necesita al menos 8 caracteres.", "Passphrase needs at least 8 characters.");
        }
        const entropy = new Uint8Array(W.phraseToEntropy($("import-phrase").value.trim().toLowerCase()));
        const blob = new Uint8Array(W.sealKeystore(entropy, enc.encode(pass), rand(16), rand(24)));
        if (!okToReplace(blob)) { entropy.fill(0); return; }
        await storeBoth(blob);
        wallet = W.Wallet.fromEntropy(entropy);
        entropy.fill(0);
        $("import-phrase").value = ""; $("import-pass").value = "";
        enterLoginScreen();
      } catch {
        status("gate-status", "err", "Esas 24 palabras no forman una máscara válida.", "Those 24 words do not form a valid mask.");
      }
    });

    // ── login ──
    function enterLoginScreen() {
      const pub = new Uint8Array(wallet.publicKey());
      $("login-handle").textContent = W.addressOf(pub);
      $("login-ss58").textContent = W.ss58Of(pub);
      show("s-login");
    }
    $("btn-lock").addEventListener("click", () => { forgetSession(); wallet?.free?.(); wallet = null; show("s-gate"); refreshGate(); });

    $("btn-login").addEventListener("click", async () => {
      const btn = $("btn-login"); btn.disabled = true;
      status("login-status", "info", "Pidiendo reto…", "Requesting challenge…");
      try {
        const pub = new Uint8Array(wallet.publicKey());
        const ss58 = W.ss58Of(pub);
        const ch = await fetch(`/api/villa/challenge?mask=${encodeURIComponent(ss58)}`, { credentials: "same-origin" });
        if (!ch.ok) throw new Error("challenge " + ch.status);
        // REAL contract of the live backend (measured 2026-08-03): { nonce, msg_hex, issued_at, ttl } —
        // the genesis_hash from the notes does NOT travel as a field: it goes EMBEDDED in msg_hex
        // (prefix || nonce(32) || genesis(32) || issued_le(8)). It is extracted structurally:
        // locate the nonce inside the message and take the following 32 bytes. If one day the
        // backend adds genesis_hash as a field, that path wins.
        const c = await ch.json();
        const nonce = unhex(c.nonce);
        let genesis = c.genesis_hash ? unhex(c.genesis_hash) : null;
        if (!genesis && c.msg_hex && nonce) {
          const msg = unhex(c.msg_hex);
          if (msg) {
            const nh = hex(nonce);
            const at = hex(msg).indexOf(nh);
            if (at >= 0 && at % 2 === 0) {
              const off = at / 2 + 32;
              if (msg.length >= off + 32) genesis = msg.slice(off, off + 32);
            }
          }
        }
        if (!nonce || nonce.length !== 32 || !genesis || genesis.length !== 32) throw new Error("bad challenge");
        status("login-status", "info", "Firmando en tu dispositivo…", "Signing on your device…");
        const sig = new Uint8Array(wallet.signVillaLogin(nonce, genesis, BigInt(c.issued_at)));
        const res = await fetch("/api/villa/login", {
          method: "POST", credentials: "same-origin",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ mask_ss58: ss58, sig: hex(sig), nonce: c.nonce }),
        });
        if (!res.ok) throw new Error("login " + res.status);
        const body = await res.json().catch(() => ({}));
        sessionExp = body.exp ?? null;
        $("shell-handle").textContent = $("login-handle").textContent;
        $("shell-exp").innerHTML = sessionExp
          ? `<span class="es" lang="es">La sesión expira sola en ~30 min; tu llave no queda en ningún servidor.</span><span class="en" lang="en">Session self-expires in ~30 min; your key rests on no server.</span>`
          : "";
        rememberSession($("login-handle").textContent);
        status("login-status", "ok", "Firma aceptada. Bienvenido a la villa.", "Signature accepted. Welcome to the villa.");
        crossThreshold();
      } catch (e) {
        status("login-status", "err",
          "No se pudo entrar: " + (e?.message ?? e) + ". Si el reto caducó (2 min), prueba otra vez.",
          "Could not enter: " + (e?.message ?? e) + ". If the challenge expired (2 min), try again.");
      } finally { btn.disabled = false; }
    });

    // ── shell ──
    $("btn-logout").addEventListener("click", async () => {
      // No server call: the mediator keeps NO session (login proves control of the mask, nothing more),
      // so there is nothing to close there. The old best-effort POST /api/villa/logout hit an endpoint that
      // never existed — a 404 in the console on every "Leave" (found by the walk, 2026-09-23).
      forgetSession();
      wallet?.free?.(); wallet = null;
      show("s-gate"); refreshGate();
    });

    // ── bind node (#837) ──
    $("bind-ack").addEventListener("change", e => $("btn-bind-verify").disabled = !e.target.checked);
    $("btn-bind-verify").addEventListener("click", () => {
      if (!wallet) return status("bind-status", "err",
        "Para esto hace falta tu máscara abierta: pulsa Salir y ábrela.", "This needs your mask open: press Leave and open it.");
      const sk = unhex($("bind-skpub").value), pop = unhex($("bind-pop").value);
      if (!sk || sk.length !== 32) return status("bind-status", "err", "sk_pub debe ser 32 bytes hex.", "sk_pub must be 32 hex bytes.");
      if (!pop || pop.length !== 64) return status("bind-status", "err", "pop_sig debe ser 64 bytes hex.", "pop_sig must be 64 hex bytes.");
      const pub = new Uint8Array(wallet.publicKey());
      if (W.verifyNodePop(sk, pub, pop)) {
        status("bind-status", "ok",
          "Prueba VÁLIDA: ese nodo posee su clave y la ató a TU máscara. Pulsa «Firmar y enviar» para dejarlo escrito en la cadena (no cuesta nada).",
          "Proof VALID: the node holds its key and bound it to YOUR mask. Press «Sign and send» to write it on the chain (it costs nothing).");
        bindReady = { sk: "0x" + hex(sk), pop: "0x" + hex(pop), replaces: undefined };
        $("btn-bind-send").disabled = false; $("btn-bind-send").removeAttribute("title");
      } else {
        status("bind-status", "err",
          "Prueba INVÁLIDA: o la firma no es de esa clave, o el nodo la ató a otra máscara. Regenera la prueba desde la consola del nodo con TU máscara en pantalla.",
          "Proof INVALID: either the signature is not from that key, or the node bound it to another mask. Regenerate the proof from the node console with YOUR mask on screen.");
      }
    });

    // #1468 (28-sep): "Sign and send" through the site's mediator (/api), never through a node's RPC (the
    // CSP forbids it and a newcomer has no node to call). Signing happens INSIDE the wasm wallet;
    // only public bytes travel here. If the mask already had a node, binding another one REPLACES it:
    // the engine (bind.js) refuses without confirmation, and the confirmation stays tied to the key shown.
    let bindReady = null;
    // status() paints with innerHTML: anything coming from outside (node error, hash, key) is escaped.
    const esc = (t) => String(t).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
    ["bind-skpub", "bind-pop"].forEach(id => $(id).addEventListener("input", () => {
      bindReady = null; $("btn-bind-send").disabled = true;
    }));
    $("btn-bind-send").addEventListener("click", async () => {
      if (!wallet || !bindReady || !window.HlqBind) return;
      const btn = $("btn-bind-send"); btn.disabled = true;
      window.HlqWallet = {
        pubkeyHex: () => wallet ? hex(new Uint8Array(wallet.publicKey())) : null,
        signExtrinsicPayload: (p) => new Uint8Array(wallet.signExtrinsicPayload(p)),
      };
      const said = {
        "reading-chain": ["Leyendo la cadena…", "Reading the chain…"],
        "about-to-sign": ["Firmando en tu dispositivo…", "Signing on your device…"],
        "submitting": ["Enviando…", "Sending…"],
      };
      try {
        const h = await window.HlqBind.claimNode({
          rpcUrl: "/api", votePubHex: bindReady.sk, proofHex: bindReady.pop,
          confirmReplace: bindReady.replaces !== undefined, expectReplaces: bindReady.replaces,
          onStep: (k) => { if (said[k]) status("bind-status", "info", said[k][0], said[k][1]); },
        });
        // 28-sep (dress rehearsal): "sent" is NOT "bound". A transaction can enter a block and the chain
        // can still not apply the binding (e.g. that node already belongs to ANOTHER mask); before, the
        // villa said "bound" regardless. Now it confirms by reading the chain: up to 6 reads, one every
        // 6 s (~3 blocks).
        const want = bindReady.sk.toLowerCase();
        const me = new Uint8Array(wallet.publicKey());
        bindReady = null;
        status("bind-status", "info",
          "Enviado (resguardo " + esc(h) + "). Comprobando en la cadena que tu nodo ha quedado vinculado…",
          "Sent (receipt " + esc(h) + "). Checking on the chain that your node is now bound…");
        let seen = null;
        for (let i = 0; i < 6 && seen !== want; i++) {
          await new Promise(r => setTimeout(r, 6000));
          try { seen = ((await window.HlqBind.currentBinding("/api", me)) || "").toLowerCase(); } catch (_) { /* retries */ }
        }
        if (seen === want) {
          status("bind-status", "ok",
            "✓ Vinculado: la cadena ya dice que ese nodo es de tu máscara. Resguardo: " + esc(h),
            "✓ Bound: the chain now says that node is yours. Receipt: " + esc(h));
        } else {
          status("bind-status", "err",
            "La transacción entró, pero la cadena NO ha aplicado el vínculo. Lo más probable: ese nodo ya está vinculado a OTRA máscara (en tu nodo: <code>sudo harlequin-link-to-mask TU_DIRECCIÓN --rotate</code> y pega los valores nuevos), o has gastado las escrituras gratuitas de este rato (espera 2 minutos y repite).",
            "The transaction went in, but the chain did NOT apply the binding. Most likely: that node is already bound to ANOTHER mask (on your node: <code>sudo harlequin-link-to-mask YOUR_ADDRESS --rotate</code> and paste the new values), or you used up this period's free writes (wait 2 minutes and retry).");
          btn.disabled = false;
        }
      } catch (e) {
        if (e.code === "would-replace" || e.code === "changed-since-review") {
          bindReady.replaces = e.currentVoteKey || null;
          status("bind-status", "err",
            "Esta máscara YA tiene un nodo vinculado (" + esc(e.currentVoteKey) + "). Si envías, el nuevo lo SUSTITUYE y el anterior deja de contar. Pulsa otra vez «Firmar y enviar» solo si es eso lo que quieres.",
            "This mask ALREADY has a node bound (" + esc(e.currentVoteKey) + "). Sending REPLACES it and the old one stops counting. Press «Sign and send» again only if that is what you want.");
          btn.disabled = false; return;
        }
        if (/rate limited/.test(e.message)) {
          status("bind-status", "err", "Demasiados intentos seguidos. Espera un minuto y vuelve a pulsar «Firmar y enviar».",
            "Too many attempts in a row. Wait a minute and press «Sign and send» again.");
        } else if (/Invalid Transaction/i.test(e.message)) {
          status("bind-status", "err",
            "La cadena rechazó el envío (" + esc(e.message) + "). Si acabas de enviar otra cosa, espera 2 minutos: una máscara nueva tiene unas pocas escrituras gratuitas cada rato. Luego vuelve a pulsar «Firmar y enviar».",
            "The chain refused it (" + esc(e.message) + "). If you just sent something else, wait 2 minutes: a new mask has a few free writes per period. Then press «Sign and send» again.");
        } else {
          status("bind-status", "err", "No se pudo enviar: " + esc(e.message), "Could not send: " + esc(e.message));
        }
        btn.disabled = false;
      } finally {
        window.HlqWallet = undefined;
      }
    });

    // The citizen's walk (23-sep-2026, harness web/walk-citizen.py): the rite seals the mask WITHOUT a
    // passphrase, but this door used to ask for one → the newcomer got stuck at "what passphrase?". If the
    // stored mask opens with the empty passphrase, it opens by itself: this is exactly what pressing
    // "Open" with the field empty already did, with no new exposure. A mask WITH a lock fails here silently
    // and the unlock screen stays as it always was.
    function openIfUnlocked() {
      const ls = localStorage.getItem(KS_KEY);
      const blob = RITO_BLOB || (ls ? unhex(ls) : null)   /* indexedDB (canonical) first — four-eyes Forge 25-sep */;
      if (!blob || wallet) return;
      const remembered = rememberedSession();
      try {
        const entropy = W.openKeystore(blob, new Uint8Array(0));
        if (!ls && RITO_BLOB) localStorage.setItem(KS_KEY, hex(RITO_BLOB)); // migrates the rite's drawer
        wallet = W.Wallet.fromEntropy(new Uint8Array(entropy));
        entropy.fill?.(0);
        if (remembered) return;       // already shown inside (boot): open the key quietly, stay there
        enterLoginScreen();
        status("login-status", "info",
          "Tu máscara no tiene frase de paso: se abre sola en este dispositivo.",
          "Your mask has no passphrase: it opens by itself on this device.");
      } catch { /* with a lock: it opens by hand, as always */ }
    }

    // ══ 1-oct-2026 — the door and the world (the maintainer: "like crossing a door into another world",
    // "almost a game") ══
    // Everything told about the villa comes from the chain or the mediator; if it does not answer, it stays
    // silent (never invented).
    const L = (es, en) => `<span class="es" lang="es">${es}</span><span class="en" lang="en">${en}</span>`;
    const REDUCED = window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches;

    function crossThreshold() {
      const portal = $("portal");
      if (REDUCED || !portal) return show("s-shell");
      portal.classList.remove("go"); void portal.offsetWidth; portal.classList.add("go");
      setTimeout(() => { show("s-shell"); $("world").classList.add("arrive"); }, 2500);   // under the flash
      setTimeout(() => portal.classList.remove("go"), 3500);
    }

    const PLACES = {
      banco:    { href: "/wallet",  es: ["El Banco", "Tu cartera: lo que tienes, lo que envías y lo que cobras. La llave es tuya; el cofre también."],
                                    en: ["The Bank", "Your wallet: what you hold, send and earn. The key is yours; so is the chest."] },
      mercado:  { href: "/market",  es: ["El Mercado", "Puestos que abren los propios vecinos: ofrece lo que sabes hacer, encuentra lo que necesitas."],
                                    en: ["The Market", "Stalls opened by the neighbours themselves: offer what you can do, find what you need."] },
      foro:     { href: "/el-foro", es: ["El Foro", "Bajo los arcos se habla en público. Lo que se dice queda escrito en la cadena."],
                                    en: ["The Forum", "Under the arches people speak in public. What is said stays written on the chain."] },
      corro:    { href: "/corro",   es: ["El Corro", "Conversaciones entre máscaras, a salvo de oídos ajenos."],
                                    en: ["The Circle", "Conversations between masks, safe from other ears."] },
      consejo:  { href: "/consejo", es: ["El Consejo", "Donde la villa se juzga a sí misma: casos, jurados y veredictos entre iguales."],
                                    en: ["The Council", "Where the villa judges itself: cases, juries and verdicts among equals."] },
      camerino: { href: "/panel",   es: ["Tu Camerino", "Tu máscara por dentro: tu reputación, tus obras y tu historia en la villa."],
                                    en: ["Your Dressing Room", "Your mask from the inside: your reputation, your deeds and your story in the villa."] },
      red:      { href: "/network", es: ["El Estandarte", "La red que sostiene la villa: nodos, bloques y quién firma, en vivo."],
                                    en: ["The Banner", "The network holding up the villa: nodes, blocks and who signs, live."] },
      campana:  { href: "/network", es: ["La Campana", "Tañe con cada bloque nuevo. Mientras suene, la villa está viva."],
                                    en: ["The Bell", "It tolls with every new block. As long as it rings, the villa is alive."] },
      forja:    { forja: true,      es: ["La Forja", ""], en: ["The Forge", ""] },
      notario:  { notario: true,    es: ["El Notario", ""], en: ["The Notary", ""] },
    };
    const LIVE = { block: null, fin: null, offers: null, posts: null, neighbours: null };

    function openPlace(key) {
      const P = PLACES[key]; if (!P) return;
      $("placeImg").className = "place-img pi-" + key;
      $("placeName").innerHTML = L(P.es[0], P.en[0]);
      const special = P.forja || P.notario;
      $("placeDesc").innerHTML = special ? "" : L(P.es[1], P.en[1]);
      $("placeLive").innerHTML = placeLive(key);
      $("placeGo").href = P.href || "#";
      document.querySelector("#place .place-body").hidden = !!special;
      $("forjaBody").hidden = !P.forja;
      $("notarioBody").hidden = !P.notario;
      $("place").hidden = false;
      (P.forja ? $("bind-ack") : P.notario ? $("pact-text") : $("placeGo")).focus({ preventScroll: true });
    }
    function closePlace() { const p = $("place"); if (p) p.hidden = true; }
    function placeLive(key) {
      if (key === "mercado" && LIVE.offers != null) return L(`Ahora mismo hay ${LIVE.offers} puesto${LIVE.offers === 1 ? "" : "s"} abierto${LIVE.offers === 1 ? "" : "s"}.`, `${LIVE.offers} stall${LIVE.offers === 1 ? " is" : "s are"} open right now.`);
      if (key === "foro" && LIVE.posts != null) return L(`${LIVE.posts} palabra${LIVE.posts === 1 ? "" : "s"} escrita${LIVE.posts === 1 ? "" : "s"} bajo los arcos.`, `${LIVE.posts} word${LIVE.posts === 1 ? "" : "s"} written under the arches.`);
      if ((key === "campana" || key === "red") && LIVE.block != null) return L(`Último tañido: bloque #${LIVE.block.toLocaleString("es")} · sellado hasta el #${LIVE.fin.toLocaleString("es")}.`, `Last toll: block #${LIVE.block.toLocaleString("en")} · sealed up to #${LIVE.fin.toLocaleString("en")}.`);
      return "";
    }

    // The square's windows: one lights up for each neighbour who has left a deed (a stall or a word). The villa grows in plain sight.
    const WINDOWS = [[56.6,25.6],[60.5,26.4],[64.1,25.3],[67.7,27.6],[71.6,23.9],[74.9,25.3],[45.6,29.6],[43,33.3],[16.3,29.6],[21.5,33],[31.2,32.1],[84.6,38.7],[68.4,31.9],[77.5,29.6]];  // houses at the back of the valley (7-oct)
    function paintWindows(n) {
      const box = $("windows"); if (!box) return;
      if (!box.children.length) box.innerHTML = WINDOWS.map(([x, y], i) => `<i class="win" data-i="${i}"></i>`).join("");
      [...box.children].forEach((w, i) => { w.style.left = WINDOWS[i][0] + "%"; w.style.top = WINDOWS[i][1] + "%";
        w.style.animationDelay = (-i * 0.37) + "s"; w.classList.toggle("lit", i < n); });
    }

    function chronicle() {
      const parts = [];
      if (LIVE.neighbours != null) parts.push(L(`${LIVE.neighbours} vecino${LIVE.neighbours === 1 ? "" : "s"} con obra`, `${LIVE.neighbours} neighbour${LIVE.neighbours === 1 ? "" : "s"} with deeds`));
      if (LIVE.offers != null) parts.push(L(`${LIVE.offers} puesto${LIVE.offers === 1 ? "" : "s"} en el mercado`, `${LIVE.offers} market stall${LIVE.offers === 1 ? "" : "s"}`));
      if (LIVE.block != null) parts.push(L(`la campana va por el bloque #${LIVE.block.toLocaleString("es")}`, `the bell is at block #${LIVE.block.toLocaleString("en")}`));
      if (parts.length) $("chrText").innerHTML = parts.join(" · ");
    }

    async function pollChain() {
      try {
        const r = await fetch("/network.json", { cache: "no-store" }); if (!r.ok) return;
        const j = await r.json();
        const b = +j.bestBlock, f = +j.finalizedBlock; if (!Number.isFinite(b)) return;
        const rang = LIVE.block != null && b > LIVE.block;
        LIVE.block = b; LIVE.fin = Number.isFinite(f) ? f : b; chronicle();
        if (rang && !REDUCED) { const c = $("chronicle"), sc = $("scene");
          [c, sc].forEach(e => { e.classList.remove("ring"); void e.offsetWidth; e.classList.add("ring"); }); }
      } catch (e) { /* the chronicle stays silent: nothing is invented */ }
    }
    async function pollSociety() {
      const who = new Set();
      try { const r = await fetch("/api/market/offers"); if (r.ok) { const o = ((await r.json()).offers || []).filter(x => !x.withdrawn);
        LIVE.offers = o.length; o.forEach(x => who.add(x.pubkey)); } } catch (e) {}
      try { const r = await fetch("/api/forum/posts"); if (r.ok) { const p = (await r.json()).posts || [];
        LIVE.posts = p.length; p.forEach(x => who.add(x.pubkey)); } } catch (e) {}
      if (LIVE.offers != null || LIVE.posts != null) { LIVE.neighbours = who.size; paintWindows(who.size); }
      chronicle();
    }

    let worldOn = false;
    function startWorld() {
      paintWindows(LIVE.neighbours || 0);
      const sc = $("sceneScroll");
      requestAnimationFrame(() => { if (sc.scrollWidth > sc.clientWidth) sc.scrollLeft = (sc.scrollWidth - sc.clientWidth) * 0.42; });
      if (worldOn) return; worldOn = true;
      pollChain(); pollSociety();
      setInterval(() => { if (document.body.classList.contains("in-villa") && !document.hidden) pollChain(); }, 15000);
      setInterval(() => { if (document.body.classList.contains("in-villa") && !document.hidden) pollSociety(); }, 90000);
    }

    document.querySelectorAll(".spot, .pb").forEach(b => b.addEventListener("click", () => openPlace(b.dataset.place)));
    $("placeClose").addEventListener("click", closePlace);
    $("place").addEventListener("click", e => { if (e.target === $("place")) closePlace(); });
    document.addEventListener("keydown", e => { if (e.key === "Escape") closePlace(); });
    // drag the square with the mouse (on mobile it already slides with a finger)
    (() => { const sc = $("sceneScroll"); let x0 = null, s0 = 0, moved = false;
      sc.addEventListener("mousedown", e => { if (e.target.closest(".spot")) return; x0 = e.clientX; s0 = sc.scrollLeft; moved = false; });
      window.addEventListener("mousemove", e => { if (x0 == null) return; const d = e.clientX - x0; if (Math.abs(d) > 3) moved = true; sc.scrollLeft = s0 - d; });
      window.addEventListener("mouseup", () => { x0 = null; });
    })();


    // ══ El Notario (1-oct-2026, design/PACTOS-CONTRATOS-PRIVADOS-2026-10-01.md) ══
    // The text never leaves the device: the package carries the document's sha256, the salt, the commitment and the
    // parties' signatures (each over `hlq-pact-sign-v1 ++ genesis ++ commitment`). Recording on chain arrives with
    // the 27-nov genesis (pallet-pacts); the signed package already proves who signed what.
    let GENESIS = null;
    $("pact-doc-text").placeholder = document.documentElement.classList.contains("lang-es") ? "…o pega el texto" : "…or paste the text";
    async function genesisHash() {
      if (GENESIS) return GENESIS;
      if (!wallet) throw new Error("mask-closed");
      const ch = await fetch(`/api/villa/challenge?mask=${encodeURIComponent(W.ss58Of(new Uint8Array(wallet.publicKey())))}`, { credentials: "same-origin" });
      if (!ch.ok) throw new Error("challenge " + ch.status);
      const c = await ch.json(), nonce = unhex(c.nonce), msg = unhex(c.msg_hex || "");
      if (c.genesis_hash) GENESIS = unhex(c.genesis_hash);
      else if (nonce && msg) { const at = hex(msg).indexOf(hex(nonce)); if (at >= 0 && at % 2 === 0) GENESIS = msg.slice(at / 2 + 32, at / 2 + 64); }
      if (!GENESIS || GENESIS.length !== 32) { GENESIS = null; throw new Error("bad challenge"); }
      return GENESIS;
    }
    const readFile = (input) => new Promise((res, rej) => {
      const f = input.files && input.files[0]; if (!f) return res(null);
      const r = new FileReader(); r.onload = () => res(new Uint8Array(r.result)); r.onerror = () => rej(r.error); r.readAsArrayBuffer(f);
    });
    let pactPkg = null, pactDownloadUrl = null;
    function offerDownload(pkg) {
      // data: URL, not blob: — the site's CSP (connect-src 'self') does not cover blob: and nothing here needs it.
      const bytes = enc.encode(JSON.stringify(pkg, null, 2));
      let bin = ""; for (const b of bytes) bin += String.fromCharCode(b);
      pactDownloadUrl = "data:application/json;base64," + btoa(bin);
      const a = $("pact-download"); a.href = pactDownloadUrl; a.download = `pacto-${pkg.commitment.slice(0, 8)}.hlqpact`; a.classList.remove("v-hide");
    }
    const pactErr = (e) => e && e.message === "mask-closed"
      ? status("pact-status", "err", "Para firmar hace falta tu máscara abierta: sal y vuelve a entrar.", "Signing needs your mask open: leave and enter again.")
      : status("pact-status", "err", "No se pudo: " + esc(e && e.message || e), "Could not: " + esc(e && e.message || e));
    document.querySelectorAll("[data-ntab]").forEach(t => t.addEventListener("click", () => {
      document.querySelectorAll("[data-ntab]").forEach(x => x.classList.toggle("active", x === t));
      ["nuevo", "firmar"].forEach(k => $("ntab-" + k).classList.toggle("v-hide", t.dataset.ntab !== k));
    }));
    $("pact-make").addEventListener("click", async () => {
      try {
        const file = await readFile($("pact-file"));
        const text = $("pact-text").value;
        const doc = file || (text.trim() ? enc.encode(text) : null);
        if (!doc) return status("pact-status", "err", "Escribe el texto o elige un documento.", "Write the text or choose a document.");
        const g = await genesisHash();
        const digest = new Uint8Array(W.documentDigest(doc)), salt = rand(32);
        const c = new Uint8Array(W.pactCommitment(salt, digest));
        const me = new Uint8Array(wallet.publicKey());
        const sig = new Uint8Array(wallet.signPact(g, c));
        pactPkg = { v: 1, kind: "hlq-pact", genesis: hex(g), doc_sha256: hex(digest), salt: hex(salt), commitment: hex(c),
                    sigs: [{ pub: hex(me), handle: W.addressOf(me), sig: hex(sig) }] };
        offerDownload(pactPkg);
        status("pact-status", "ok",
          `Sellado y firmado por ti. Huella <span class="pact-sig">${hex(c)}</span>. Descarga el paquete y mándaselo a la otra parte <strong>con el texto</strong> por el corro: ella lo comprueba y firma.`,
          `Sealed and signed by you. Fingerprint <span class="pact-sig">${hex(c)}</span>. Download the package and send it to the other party <strong>with the text</strong> through the circle: they check it and sign.`);
      } catch (e) { pactErr(e); }
    });
    $("pact-verify").addEventListener("click", async () => {
      try {
        const raw = await readFile($("pact-pkg"));
        if (!raw) return status("pact-status", "err", "Elige el paquete .hlqpact.", "Choose the .hlqpact package.");
        const pkg = JSON.parse(new TextDecoder().decode(raw));
        if (pkg.kind !== "hlq-pact" || pkg.v !== 1) throw new Error("not a pact package");
        const salt = unhex(pkg.salt), digest = unhex(pkg.doc_sha256), g = unhex(pkg.genesis), c = unhex(pkg.commitment);
        if (!salt || salt.length !== 32 || !digest || digest.length !== 32 || !g || g.length !== 32 || !c || c.length !== 32) throw new Error("malformed package");
        const lines = [];
        const okC = hex(new Uint8Array(W.pactCommitment(salt, digest))) === hex(c);
        lines.push(okC ? L("✓ La huella cuadra con la sal y el documento declarados.", "✓ The fingerprint matches the declared salt and document.")
                       : L("✗ La huella NO cuadra: el paquete está alterado.", "✗ The fingerprint does NOT match: the package was altered."));
        const file = await readFile($("pact-doc"));
        const pasted = $("pact-doc-text").value;
        const doc = file || (pasted.trim() ? enc.encode(pasted) : null);
        if (doc) lines.push(hex(new Uint8Array(W.documentDigest(doc))) === pkg.doc_sha256
          ? L("✓ Es ESTE texto, byte a byte.", "✓ It is THIS text, byte for byte.")
          : L("✗ Ese texto NO es el del pacto (aunque sea un espacio).", "✗ That text is NOT the pact's (even one space counts)."));
        let allSig = okC;
        for (const s of (pkg.sigs || [])) {
          const ok = W.verifyPact(unhex(s.pub), g, c, unhex(s.sig));
          allSig = allSig && ok;
          lines.push((ok ? "✓ " : "✗ ") + L(`Firma de ${esc(W.addressOf(unhex(s.pub)))}`, `Signature of ${esc(W.addressOf(unhex(s.pub)))}`));
        }
        pactPkg = okC ? pkg : null;
        const mine = wallet && (pkg.sigs || []).some(s => s.pub === hex(new Uint8Array(wallet.publicKey())));
        $("pact-cosign").disabled = !(okC && wallet && !mine);
        if (okC && pkg.sigs && pkg.sigs.length > 1) {
          const sealed = hex(new Uint8Array(W.pactSealedKey(c, unhex(pkg.sigs.map(s => s.sig).join("")))));
          lines.push(L(`Clave sellada (lo que va a la cadena en modo oculto): <span class="pact-sig">${sealed}</span>`,
                       `Sealed key (what goes on chain in hidden mode): <span class="pact-sig">${sealed}</span>`));
          // 7-oct-2026 (#1500): is it already sealed? and can this chain take it? (Pacts.seal from the runtime metadata)
          sealKey = allSig ? sealed : null;
          const at = await sealedAt(sealed);
          if (at != null) lines.push(L(`✓ Sellado en la cadena en el bloque #${at.toLocaleString("es")}.`, `✓ Sealed on the chain at block #${at.toLocaleString("en")}.`));
          const call = at == null && allSig ? await pactCall("seal") : null;
          $("pact-seal").classList.toggle("v-hide", !call);
          $("pact-seal").disabled = !(call && wallet);
        } else { sealKey = null; $("pact-seal").classList.add("v-hide"); }
        $("pact-status").className = "v-status " + (allSig ? "ok" : "err");
        $("pact-status").innerHTML = lines.join("<br>");
        $("pact-status").scrollIntoView({ block: "nearest", behavior: "smooth" });   // the verdict sits below the fold
      } catch (e) { pactErr(e); }
    });
    // ── Sealing on chain (mode B, pallet-pacts `seal`): only the key and its block are public — never the text, the
    //    salt or who signed. Signed HERE with the open mask; "sealed" only when the chain shows the key. ──
    let sealKey = null, capsCache = null;
    async function pactCall(name) {
      try {
        if (!capsCache) capsCache = await (await fetch("/api/capabilities", { cache: "no-store" })).json();
        const p = capsCache && capsCache.pallets && capsCache.pallets.Pacts;
        return p && Number.isInteger(p.calls[name]) ? [p.index, p.calls[name]] : null;
      } catch (e) { return null; }
    }
    async function sealedAt(keyHex) {
      try { const j = await (await fetch("/api/pacts/sealed?key=0x" + keyHex, { cache: "no-store" })).json(); return j.sealed_at ?? null; }
      catch (e) { return null; }
    }
    async function submitCall(call) {
      const B = window.HlqBind._internals, acc = "0x" + hex(new Uint8Array(wallet.publicKey()));
      const get = async (u) => { const r = await fetch(u, { cache: "no-store" }); const j = await r.json(); if (!r.ok) throw new Error(j.error || r.status); return j; };
      const m0 = await get("/api/tx/meta?account=" + acc), era = B.eraFor(m0.best_number);
      const m = await get("/api/tx/meta?account=" + acc + "&birth=" + era.birth);
      const ctx = { specVersion: m.spec_version, txVersion: m.tx_version, genesis: B.hexToBytes(m.genesis_hash),
                    nonce: Number(m.nonce), eraBytes: era.bytes, eraBlockHash: B.hexToBytes(m.birth_hash) };
      const sig = new Uint8Array(wallet.signExtrinsicPayload(B.signingPayload(call, ctx)));
      const xt = B.assembleExtrinsic(call, new Uint8Array(wallet.publicKey()), sig, ctx.nonce, ctx.eraBytes);
      const r = await fetch("/api/tx", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ xt: B.bytesToHex(xt) }) });
      const j = await r.json(); if (!r.ok) throw new Error(j.error || r.status); return j.hash;
    }
    $("pact-seal").addEventListener("click", async () => {
      const btn = $("pact-seal");
      try {
        if (!sealKey || !wallet || !window.HlqBind) return;
        const ix = await pactCall("seal"); if (!ix) return;
        btn.disabled = true;
        status("pact-status", "info", "Tu máscara firma el sello en este dispositivo…", "Your mask signs the seal on this device…");
        $("pact-status").scrollIntoView({ block: "nearest", behavior: "smooth" });
        await submitCall(new Uint8Array([ix[0], ix[1], ...unhex(sealKey)]));
        status("pact-status", "info", "Enviado. Esperando a que la cadena lo apunte…", "Sent. Waiting for the chain to record it…");
        for (let i = 0; i < 15; i++) {
          await new Promise(r => setTimeout(r, 4000));
          const at = await sealedAt(sealKey);
          if (at != null) {
            btn.classList.add("v-hide");
            return status("pact-status", "ok", `Sellado en la cadena en el bloque #${at.toLocaleString("es")}. Queda la fecha; el texto y las firmas siguen solo en vuestro paquete.`,
                                                `Sealed on the chain at block #${at.toLocaleString("en")}. The date stays; the text and the signatures remain only in your package.`);
          }
        }
        btn.disabled = false;
        status("pact-status", "err", "La cadena no lo ha apuntado en un minuto: puede que lo rechazara. Vuelve a comprobar el paquete.", "The chain has not recorded it within a minute: it may have rejected it. Verify the package again.");
      } catch (e) { btn.disabled = false; pactErr(e); }
    });
    $("pact-cosign").addEventListener("click", async () => {
      try {
        if (!pactPkg) return;
        const g = await genesisHash();
        if (hex(g) !== pactPkg.genesis) throw new Error("this pact belongs to another chain");
        const me = new Uint8Array(wallet.publicKey());
        pactPkg.sigs.push({ pub: hex(me), handle: W.addressOf(me), sig: hex(new Uint8Array(wallet.signPact(g, unhex(pactPkg.commitment)))) });
        offerDownload(pactPkg); $("pact-cosign").disabled = true;
        status("pact-status", "ok", "Firmado. Descarga el paquete: ahora lleva las dos firmas.", "Signed. Download the package: it now carries both signatures.");
      } catch (e) { pactErr(e); }
    });

    // ── boot ──
    const back = rememberedSession();
    if (back) {                                   // shown inside at once, before the wasm even loads
      $("shell-handle").textContent = back.h;
      $("shell-exp").innerHTML = `<span class="es" lang="es">Sesión recordada en esta pestaña hasta que la cierres o salgas.</span><span class="en" lang="en">Session remembered in this tab until you close it or leave.</span>`;
      show("s-shell");
    }
    init().then(() => { refreshGate(); return ritoBlob(); }).then((b) => { RITO_BLOB = b; refreshGate(); openIfUnlocked(); }).catch(e => {
      status("gate-status", "err", "No se pudo cargar el módulo de firma: " + e, "Could not load the signing module: " + e);
    });

