/* Harlequin — The Threshold (rite). Externalized from the inline <script> because the site's CSP
   (script-src 'self') blocks the script AND inline onclick= → here the navigation is wired with
   addEventListener over data-attributes (data-go / data-act), never onclick. REAL crypto:
   window.HlqWallet.create("") forges the 24-word BIP39 seed in the browser (loading wallet-core.js
   over the vetted wasm); the server NEVER sees it. handle = "hlq-"+base32(sha256(pubkey)). */
(function(){
  "use strict";
  let SEED=null, MASK_ADDR=null, VERIFY_IDX=[], WALLET_READY=false;
  // Language (2026-09-25): English primary, Spanish as transcription. lang-boot.js decides by domain.
  const ES=()=>document.documentElement.classList.contains('lang-es');
  const T=(es,en)=>ES()?es:en;
  document.title=T('Harlequin · El Umbral','Harlequin · The Threshold');
  const setPh=(id,es,en)=>{ const e=document.getElementById(id); if(e) e.placeholder=T(es,en); };
  setPh('lockPass','frase de paso','passphrase'); setPh('lockPass2','repítela','repeat it');
  window.addEventListener('hlqwallet-ready', ()=>{ WALLET_READY=true; }, {once:true});
  if (window.HlqWallet) WALLET_READY=true;
  const acts=[...document.querySelectorAll('.act')];

  function go(n){
    acts.forEach(a=>a.classList.remove('on'));
    acts[n].classList.add('on');
    const p=document.getElementById('progress'); if(p) p.style.width=(n/(acts.length-1)*100)+'%';
    window.scrollTo(0,0);   // instant: "smooth" left the new act halfway up the screen (1-oct)
    if(n===2) forgeSeed();
    if(n===3) buildVerify();
    if(n===4) revealMask();
  }

  // Act IV (2026-08-03): the mask IN PLAIN SIGHT. It used to be born and stay in the variable — the rite
  // ended without ever showing it (the maintainer's complaint, literally: "it just gives me the words and
  // that's it").
  function revealMask(){
    const el=document.getElementById('maskHandle');
    if(el) el.textContent = MASK_ADDR || T('— (la máscara se forja en el acto de la semilla)','— (the mask is forged in the act of the seed)');
  }
  function restart(){ SEED=null; go(0); }

  async function forgeSeed(){
    if(SEED) return;
    const grid=document.getElementById('seedGrid'), note=document.getElementById('seedNote');
    if(!window.HlqWallet || !WALLET_READY){
      grid.innerHTML='<div class="seed-word full">'+T('Forjando la cripto…','Forging the crypto…')+'</div>';
      await new Promise(res=>{ if(window.HlqWallet&&WALLET_READY) return res();
        window.addEventListener('hlqwallet-ready',res,{once:true}); });
    }
    try{
      const r=await window.HlqWallet.create("");   // empty passphrase (v1, no lock); 100% on-device
      // 2026-09-25: never show a mask that was not actually stored on this device (it used to be able to
      // "seal" without writing anything, and the newcomer would lose it on closing the tab).
      if(!(await window.HlqWallet.hasKeystore())) throw new Error(T('No se pudo guardar la máscara en este navegador. Prueba a recargar o usa otro navegador.','Could not store the mask in this browser. Try reloading, or use another browser.'));
      SEED=r.mnemonic.trim().split(/\s+/); MASK_ADDR=r.handle;
      grid.innerHTML=SEED.map((w,i)=>`<div class="seed-word"><i>${String(i+1).padStart(2,'0')}</i>${w}</div>`).join('');
    }catch(e){
      grid.innerHTML='<div class="seed-word full">'+T('Error forjando la máscara','Error forging the mask')+'</div>';
      if(note){ note.className='hint bad'; note.textContent=String(e&&e.message||e); }
    }
  }

  async function buildVerify(){
    if(!SEED) await forgeSeed();
    if(!SEED) return;
    const idx=new Set();
    while(idx.size<3) idx.add(Math.floor(Math.random()*SEED.length));
    VERIFY_IDX=[...idx].sort((a,b)=>a-b);
    const row=document.getElementById('verifyRow');
    row.innerHTML=VERIFY_IDX.map(i=>`<div><span class="verify-label">${T('palabra','word')} ${i+1}</span><input data-i="${i}" autocomplete="off" spellcheck="false"></div>`).join('');
    document.getElementById('verifyHint').innerHTML='&nbsp;';
  }

  function verifySeed(){
    const inputs=[...document.querySelectorAll('#verifyRow input')];
    const ok=inputs.every(inp=>inp.value.trim().toLowerCase()===SEED[+inp.dataset.i].toLowerCase());
    const hint=document.getElementById('verifyHint');
    if(ok){ hint.className='hint good'; hint.textContent=T('Sellada. Tu máscara es tuya y de nadie más.','Sealed. Your mask is yours and no one else\'s.'); setTimeout(()=>go(4),900); }
    else { hint.className='hint bad'; hint.textContent=T('No coinciden. Vuelve a mirar tus palabras — nadie te las puede recordar.','They do not match. Look at your words again — no one can remind you of them.'); }
  }

  // Optional and recommended lock (2026-09-23). The rite seals WITHOUT a passphrase so nobody is locked
  // out in their first minute; here it offers to re-seal it WITH a passphrase while the seed is still in
  // memory (HlqWallet.restore re-seals the SAME secret: the mask does not change). The two-drawers trap:
  // the villa copies the rite's seal into localStorage the first time it opens; that copy WITHOUT a lock
  // would keep opening itself on its own. It is deleted, but ONLY if it is byte-for-byte the old rite seal
  // (never another mask).
  function readRitoBlobHex(){
    return new Promise(res=>{
      try{
        const r=indexedDB.open('hlq-wallet',2);   // same version as wallet-core.js (2026-09-25)
        r.onupgradeneeded=()=>{ if(!r.result.objectStoreNames.contains('keystore')) r.result.createObjectStore('keystore'); };
        r.onerror=()=>res(null);
        r.onsuccess=()=>{ try{ const db=r.result;
          if(!db.objectStoreNames.contains('keystore')){ db.close(); return res(null); }
          const g=db.transaction('keystore','readonly').objectStore('keystore').get('default');
          g.onsuccess=()=>{ db.close(); res(g.result ? [...new Uint8Array(g.result)].map(b=>b.toString(16).padStart(2,'0')).join('') : null); };
          g.onerror=()=>{ db.close(); res(null); };
        }catch(e){ res(null); } };
      }catch(e){ res(null); }
    });
  }
  async function lockMask(){
    const p1=document.getElementById('lockPass'), p2=document.getElementById('lockPass2'), hint=document.getElementById('lockHint');
    const bad=(t)=>{ hint.className='hint bad'; hint.textContent=t; };
    if(!SEED) return bad(T('La semilla ya no está en memoria: pon el cerrojo desde la villa, recuperando con tus 24 palabras.','The seed is no longer in memory: set the lock from the villa, recovering with your 24 words.'));
    if(p1.value.length<8) return bad(T('La frase de paso necesita 8 caracteres o más.','The passphrase needs 8 characters or more.'));
    if(p1.value!==p2.value) return bad(T('Las dos frases no coinciden.','The two passphrases do not match.'));
    try{
      const oldHex=await readRitoBlobHex();
      await window.HlqWallet.restore(SEED.join(' '), p1.value);
      try{ if(oldHex && localStorage.getItem('hlq.keystore.v1')===oldHex) localStorage.removeItem('hlq.keystore.v1'); }catch(e){}
      p1.value=''; p2.value=''; p1.disabled=true; p2.disabled=true;
      hint.className='hint good'; hint.textContent=T('Cerrojo puesto. Desde ahora tu máscara solo se abre con tu frase de paso.','Lock set. From now on your mask only opens with your passphrase.');
    }catch(e){ bad(T('No se pudo poner el cerrojo: ','Could not set the lock: ')+(e&&e.message||e)); }
  }

  // CSP-safe wiring: zero inline onclick. Buttons carry data-go="N" or data-act="verify|restart".
  document.addEventListener('click', (ev)=>{
    const t=ev.target.closest('[data-go],[data-act]'); if(!t) return;
    if(t.hasAttribute('data-go')) return go(+t.getAttribute('data-go'));
    const a=t.getAttribute('data-act');
    if(a==='verify') return verifySeed();
    if(a==='restart') return restart();
    if(a==='lock') return lockMask();
  });

  // deep-link / QA: #0..#4 jumps to the act (for headless verification against the real CSP).
  if(location.hash){ const n=+location.hash.slice(1); if(n>=0&&n<acts.length) go(n); }
})();
