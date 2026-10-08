/* Station 5 — claim a node for your mask, from the browser.
 *
 * The whole point of this file is what it does NOT do: it never sees a seed phrase. The mask signs
 * inside the wallet wasm (`HlqWallet.signExtrinsicPayload`, the same primitive used to cold-sign the
 * Key ceremony) and only a 64-byte signature comes back. Everything here is public data: a node's
 * vote PUBLIC key, a proof of possession that only works for one mask, and chain constants read from
 * an RPC the person chooses.
 *
 * No third-party library. The standard way to build an extrinsic in a browser is a CDN bundle, and
 * this site forbids anything that phones home, so the ~120 lines of SCALE encoding this one call
 * needs are written out below. They mirror, byte for byte, the tool that assembled the runtime
 * upgrade the live chain already accepted — the shape is not guesswork.
 *
 * WARNING carried through the UI, not buried here: claiming is a ROTATION. A mask can hold exactly
 * one node key, so signing a new claim releases whatever was bound before. That is how someone could
 * be talked into unbinding their own node with values handed to them by a stranger, which is why the
 * page reads the current binding from the chain and says so on the button itself.
 */
(function () {
  "use strict";

  // Reputation is pallet 1 in the live runtime; set_vote_key is call 8 (pallet::call_index(8)).
  var PALLET_REPUTATION = 1;
  var CALL_SET_VOTE_KEY = 8;

  // ---------- tiny hex/SCALE helpers (no dependencies, on purpose) ----------

  function hexToBytes(h) {
    h = String(h || "").trim().replace(/^0x/, "");
    if (h.length % 2 !== 0 || /[^0-9a-fA-F]/.test(h)) throw new Error("not hex");
    var out = new Uint8Array(h.length / 2);
    for (var i = 0; i < out.length; i++) out[i] = parseInt(h.substr(i * 2, 2), 16);
    return out;
  }

  function bytesToHex(b) {
    var s = "";
    for (var i = 0; i < b.length; i++) s += b[i].toString(16).padStart(2, "0");
    return "0x" + s;
  }

  function concat(parts) {
    var n = 0, i;
    for (i = 0; i < parts.length; i++) n += parts[i].length;
    var out = new Uint8Array(n), off = 0;
    for (i = 0; i < parts.length; i++) { out.set(parts[i], off); off += parts[i].length; }
    return out;
  }

  /** SCALE compact encoding for the small integers used here (nonce, tip, vector lengths). */
  function compact(n) {
    if (n < 0) throw new Error("compact: negative");
    if (n < 64) return new Uint8Array([n << 2]);
    if (n < 16384) { var v = (n << 2) | 0x01; return new Uint8Array([v & 0xff, (v >> 8) & 0xff]); }
    if (n < 1073741824) {
      var w = (n * 4 + 2);
      return new Uint8Array([w & 0xff, (w >>> 8) & 0xff, (w >>> 16) & 0xff, (w >>> 24) & 0xff]);
    }
    throw new Error("compact: too large for this page");
  }

  function u32le(n) {
    return new Uint8Array([n & 0xff, (n >>> 8) & 0xff, (n >>> 16) & 0xff, (n >>> 24) & 0xff]);
  }

  // ---------- RPC ----------

  function rpc(url, method, params) {
    return fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ id: 1, jsonrpc: "2.0", method: method, params: params || [] }),
      cache: "no-store"
    })
      .then(function (r) { if (!r.ok) throw new Error("rpc http " + r.status); return r.json(); })
      .then(function (j) {
        if (j.error) throw new Error("rpc: " + (j.error.message || "error"));
        return j.result;
      });
  }

  /* Everything the signature commits to, read FRESH from the chain the user is talking to — never
   * cached and never taken from the page. The spec/tx versions have already changed twice in one
   * week, the nonce must be current or the claim can be replayed, and the genesis hash is what
   * confines the signature to this chain. */
  /* The nonce is asked by PUBLIC KEY through the runtime (AccountNonceApi), not with
   * system_accountNextIndex(address): that one needs an SS58 string, and the node rejects every
   * Harlequin (1728) address — measured 2026-09-23 on the live node ("Unknown SS58 address format").
   * The page also used to pass the hlq-… handle there, which no node understands. */
  /* #1468 (28-sep): the page cannot reach a node's RPC (the CSP forbids it, and a stranger has no node
   * URL to give). With url "/api" everything goes through the site's own mediator instead: the same
   * public chain data, same-origin. Any other url keeps talking JSON-RPC to that node, as before. */
  function isApi(url) { return url === "/api" || /\/api$/.test(url); }
  function api(url, init) {
    return fetch(url, init || { headers: { "Accept": "application/json" } }).then(function (r) {
      return r.json().catch(function () { return {}; }).then(function (j) {
        if (!r.ok) throw new Error(j.error || ("api http " + r.status));
        return j;
      });
    });
  }
  function chainContextApi(url, accountHex) {
    var q = url + "/tx/meta?account=0x" + String(accountHex).replace(/^0x/, "");
    return api(q).then(function (m) {
      var era = eraFor(m.best_number);
      return api(q + "&birth=" + era.birth).then(function (m2) {
        return {
          specVersion: m2.spec_version,
          txVersion: m2.tx_version,
          genesis: hexToBytes(m2.genesis_hash),
          nonce: Number(m2.nonce),          // system_accountNextIndex: counts what is already in the pool
          head: m.best_number,
          eraBytes: era.bytes,
          eraBirth: era.birth,
          eraBlockHash: hexToBytes(m2.birth_hash),
          validForBlocks: ERA_PERIOD
        };
      });
    });
  }

  function chainContext(url, accountHex) {
    if (isApi(url)) return chainContextApi(url, accountHex);
    return Promise.all([
      rpc(url, "state_getRuntimeVersion", []),
      rpc(url, "chain_getBlockHash", [0]),
      rpc(url, "state_call", ["AccountNonceApi_account_nonce", "0x" + String(accountHex).replace(/^0x/, "")]),
      rpc(url, "chain_getHeader", [])
    ]).then(function (r) {
      var nb = hexToBytes(r[2]);                      // u32 little-endian
      r[2] = (nb[0] | (nb[1] << 8) | (nb[2] << 16) | (nb[3] << 24)) >>> 0;
      var head = parseInt(r[3].number, 16);
      var era = eraFor(head);
      return rpc(url, "chain_getBlockHash", [era.birth]).then(function (birthHash) {
        return {
          specVersion: r[0].specVersion,
          txVersion: r[0].transactionVersion,
          genesis: hexToBytes(r[1]),
          nonce: Number(r[2]),
          head: head,
          eraBytes: era.bytes,
          eraBirth: era.birth,
          eraBlockHash: hexToBytes(birthHash),
          validForBlocks: ERA_PERIOD
        };
      });
    });
  }

  /** Who, if anyone, this mask has already claimed. Read from the chain, never from the page. */
  function currentBinding(url, account32) {
    var read = isApi(url)
      ? api(url + "/consensus/vote_keys").then(function (j) { return j.hex; })
      : rpc(url, "state_call", ["HarlequinConsensusApi_vote_keys", "0x"]);
    return read.then(function (hex) {
      var b = hexToBytes(hex);
      // Vec<([u8;32], [u8;32])>: compact length, then fixed 64-byte pairs. Only the single-byte
      // length form is expected here (the map holds delegating members only), but read it properly.
      var off = 0, len;
      var first = b[0];
      if ((first & 0x03) === 0) { len = first >> 2; off = 1; }
      else if ((first & 0x03) === 1) { len = ((b[0] | (b[1] << 8)) >> 2); off = 2; }
      else { len = ((b[0] | (b[1] << 8) | (b[2] << 16) | (b[3] << 24)) >>> 2); off = 4; }
      for (var i = 0; i < len; i++) {
        var acc = b.slice(off, off + 32), key = b.slice(off + 32, off + 64);
        off += 64;
        var same = true;
        for (var k = 0; k < 32; k++) if (acc[k] !== account32[k]) { same = false; break; }
        if (same) return bytesToHex(key);
      }
      return null;
    });
  }

  // ---------- the call, the payload, the extrinsic ----------

  function buildCall(votePub32, proof64) {
    if (votePub32.length !== 32) throw new Error("the vote key must be 32 bytes");
    if (proof64.length !== 64) throw new Error("the proof must be 64 bytes");
    return concat([new Uint8Array([PALLET_REPUTATION, CALL_SET_VOTE_KEY]), votePub32, proof64]);
  }

  /* A MORTAL era, on purpose. The ceremony tool signs immortal because a governance signature is
   * assembled and used in one sitting under supervision; a claim signed in someone's browser is
   * different — an immortal signature can be kept and replayed months later, and since claiming is a
   * rotation, a stale one could silently re-bind a node its owner had already replaced. This one
   * stops being valid after `ERA_PERIOD` blocks (~13 min at 12 s).
   *
   * Encoding mirrors sp_runtime's Era::mortal, verified byte-for-byte against the runtime types
   * (node/tools/encode-probe), including the birth block the signature is anchored to. */
  var ERA_PERIOD = 64;

  function eraFor(current, period) {
    period = period || ERA_PERIOD;
    // period → power of two, clamped exactly like the runtime does
    var p = 4;
    while (p < period) p *= 2;
    p = Math.min(Math.max(p, 4), 65536);
    var phase = current % p;
    var quantizeFactor = Math.max(1, p >> 12);
    var quantizedPhase = Math.floor(phase / quantizeFactor) * quantizeFactor;
    var trailing = Math.log2(p) | 0;
    var encoded = Math.min(15, Math.max(1, trailing - 1)) | ((quantizedPhase / quantizeFactor) << 4);
    return {
      bytes: new Uint8Array([encoded & 0xff, (encoded >> 8) & 0xff]),
      // birth: the block this era hangs from — its hash goes into the signed payload
      birth: (Math.floor((Math.max(current, quantizedPhase) - quantizedPhase) / p)) * p + quantizedPhase
    };
  }

  /* Signed extensions, in the runtime's order. Only three carry bytes: era, nonce (compact) and tip
   * (compact 0). The rest are empty by construction. */
  function extensionBytes(nonce, eraBytes) {
    return concat([eraBytes || new Uint8Array([0x00]), compact(nonce), compact(0)]);
  }

  /* The implicit half: spec version, tx version, genesis, and the hash of the era's birth block
   * (genesis itself when the era is immortal). The genesis hash is what stops this signature from
   * being valid on any other chain that shares the format. */
  function implicitBytes(ctx) {
    return concat([
      u32le(ctx.specVersion), u32le(ctx.txVersion), ctx.genesis, ctx.eraBlockHash || ctx.genesis
    ]);
  }

  function signingPayload(call, ctx) {
    return concat([call, extensionBytes(ctx.nonce, ctx.eraBytes), implicitBytes(ctx)]);
  }

  /** v4 signed extrinsic: length ‖ 0x84 ‖ MultiAddress::Id(account) ‖ Sr25519(sig) ‖ extensions ‖ call. */
  function assembleExtrinsic(call, account32, signature64, nonce, eraBytes) {
    var body = concat([
      new Uint8Array([0x84]),
      new Uint8Array([0x00]), account32,
      new Uint8Array([0x01]), signature64,
      extensionBytes(nonce, eraBytes),
      call
    ]);
    return concat([compact(body.length), body]);
  }

  // ---------- the one exported flow ----------

  /**
   * Claim `votePubHex` for the unlocked mask. Returns the extrinsic hash on success.
   * `onStep(stateKey, detail)` is called at each stage so the page can narrate in the user's own
   * language — this module deliberately holds no user-facing strings.
   */
  function claimNode(opts) {
    var url = opts.rpcUrl;
    var votePub = hexToBytes(opts.votePubHex);
    var proof = hexToBytes(opts.proofHex);
    var step = opts.onStep || function () {};

    if (!window.HlqWallet) return Promise.reject(new Error("wallet not loaded"));
    var accountHex = window.HlqWallet.pubkeyHex && window.HlqWallet.pubkeyHex();
    if (!accountHex) return Promise.reject(new Error("locked"));
    var account = hexToBytes(accountHex);

    step("reading-chain");
    // The rotation warning is enforced HERE, not left to the page. Claiming releases whatever this
    // mask had bound before, so a stranger's two values can talk someone into unbinding their own
    // node. A page can be rewritten a year from now by someone who never heard that; the engine
    // refusing to sign without an explicit acknowledgement cannot be forgotten.
    return Promise.all([chainContext(url, accountHex), currentBinding(url, account)])
      .then(function (both) {
        var ctx = both[0], already = both[1];
        if (already && !opts.confirmReplace) {
          var err = new Error("this mask already has a node bound");
          err.code = "would-replace";
          err.currentVoteKey = already;
          throw err;
        }
        // Time passes between "review" and "sign". The nonce is re-read anyway, but the BINDING can
        // change too — and the person consented to replacing a specific key, not whatever is there
        // now. If the caller told us what it showed, hold it to that.
        if (opts.expectReplaces !== undefined && (opts.expectReplaces || null) !== (already || null)) {
          var e2 = new Error("the binding changed since it was shown");
          e2.code = "changed-since-review";
          e2.currentVoteKey = already;
          throw e2;
        }
        ctx.replaces = already;
        return ctx;
      })
      .then(function (ctx) {
      var call = buildCall(votePub, proof);
      var payload = signingPayload(call, ctx);
      // "What you see is what you sign", enforced rather than promised: if the page showed the exact
      // bytes and the chain has moved since (a new nonce, a new era window), the bytes about to be
      // signed are NOT the ones the person read — so refuse and make them look again.
      if (opts.expectPayloadHex && bytesToHex(payload) !== opts.expectPayloadHex) {
        var pe = new Error("the bytes changed since they were shown");
        pe.code = "payload-changed";
        throw pe;
      }
      // Hand the page exactly what is about to be signed, so "what you see is what you sign" is
      // checkable rather than promised.
      step("about-to-sign", {
        account: accountHex,
        voteKey: bytesToHex(votePub),
        replaces: ctx.replaces || null,   // non-null ⇒ the page MUST say "this replaces …"
        specVersion: ctx.specVersion,
        nonce: ctx.nonce,
        validForBlocks: ctx.validForBlocks,
        payload: bytesToHex(payload)
      });
      // Substrate signs a payload as-is up to 256 bytes and signs its blake2-256 hash beyond that.
      // This one is ~174 bytes today, and the >256 branch lives INSIDE the wallet wasm (documented
      // in wallet-core.js), so it is handled — but if a future runtime adds a signed extension the
      // payload could cross the line silently, and the two sides must agree on which rule applied.
      // Left visible rather than assumed: a page that logs this once is cheaper than a week of
      // "valid signature, rejected extrinsic".
      if (payload.length > 256 && typeof console !== "undefined") {
        console.warn("hlq-bind: payload " + payload.length + " bytes — over the 256 threshold; the wallet hashes it before signing");
      }
      var sig = window.HlqWallet.signExtrinsicPayload(payload);
      if (!sig || sig.length !== 64) throw new Error("the wallet did not return a signature");
      var xt = assembleExtrinsic(call, account, sig, ctx.nonce, ctx.eraBytes);
      step("submitting");
      if (isApi(url)) {
        return api(url + "/tx", {
          method: "POST",
          headers: { "Content-Type": "application/json", "Accept": "application/json" },
          body: JSON.stringify({ xt: bytesToHex(xt) })
        }).then(function (j) { return j.hash; });
      }
      return rpc(url, "author_submitExtrinsic", [bytesToHex(xt)]);
    }).then(function (hash) {
      step("submitted", { hash: hash });
      return hash;
    });
  }

  /** Build exactly what claimNode would sign, without signing: lets a page SHOW the bytes first. */
  function previewPayload(url, accountHex, votePubHex, proofHex) {
    return chainContext(url, accountHex).then(function (ctx) {
      var call = buildCall(hexToBytes(votePubHex), hexToBytes(proofHex));
      return { payloadHex: bytesToHex(signingPayload(call, ctx)), ctx: ctx };
    });
  }

  window.HlqBind = {
    claimNode: claimNode,
    currentBinding: currentBinding,
    previewPayload: previewPayload,
    // exported for the page's own sanity checks and for tests; all pure, all public data
    // Pure, public-data helpers, exported so the encoding can be DIFFED against the runtime's own
    // output instead of trusted (see node/tools/encode-probe). Nothing here touches a secret.
    _internals: {
      hexToBytes: hexToBytes, bytesToHex: bytesToHex, compact: compact, buildCall: buildCall,
      extensionBytes: extensionBytes, implicitBytes: implicitBytes, eraFor: eraFor,
      signingPayload: signingPayload, assembleExtrinsic: assembleExtrinsic
    }
  };
})();
