//! harlequin-pact — seal, sign and verify Harlequin pacts from a terminal (2026-10-01).
//!
//! Same package format as the villa's Notary (`.hlqpact`, JSON), so a pact signed in the browser verifies here and
//! the other way round. The document never leaves your machine; the package carries only its sha256, the salt,
//! the salted commitment and the parties' signatures (design/PACTOS-CONTRATOS-PRIVADOS-2026-10-01.md).
//!
//!   harlequin-pact new    <document> <genesis_hex> > pact.hlqpact    # seal + sign (24 words on stdin)
//!   harlequin-pact sign   <pact.hlqpact> > pact2.hlqpact             # add your signature (24 words on stdin)
//!   harlequin-pact verify <pact.hlqpact> [document]                  # check commitment, text and signatures
//!
//! The 24 words are read from stdin and never written anywhere. Pipe them in (`< file`, then destroy the file):
//! the tool REFUSES to read them from an interactive terminal, where typing them would echo on screen and stay in
//! the scrollback. A package with a repeated field is refused (the browser would read the last copy, a naive reader
//! the first: both tools must see the same pact). Exit code 0 only if everything checks.
//! Build: cargo build --release --example harlequin-pact (in chain/wallet-core).
use std::io::{IsTerminal, Read};
use wallet_core::{
    address_of, document_digest, mnemonic::phrase_to_entropy, pact_commitment, pact_sealed_key, Wallet,
};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim().trim_start_matches("0x");
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}
fn arr32(v: &[u8]) -> Option<[u8; 32]> {
    v.try_into().ok()
}
fn die(msg: &str) -> ! {
    eprintln!("harlequin-pact: {msg}");
    std::process::exit(2)
}
fn wallet_from_stdin() -> Wallet {
    if std::io::stdin().is_terminal() {
        die("refusing to read the 24 words from a terminal (they would echo on screen): pipe them in, e.g. `< words.txt`");
    }
    let mut words = String::new();
    std::io::stdin().read_to_string(&mut words).unwrap_or_else(|_| die("could not read the 24 words from stdin"));
    let entropy = phrase_to_entropy(words.trim()).unwrap_or_else(|_| die("those words do not form a valid mask"));
    Wallet::from_entropy(&entropy).unwrap_or_else(|_| die("bad mask"))
}
fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| die(&format!("cannot read {path}: {e}")))
}

/// Minimal JSON for our own flat package (no external dependency): we write it ourselves and parse it strictly.
fn field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let k = format!("\"{key}\"");
    // Top-level fields must appear exactly once: a duplicate would make this reader and JSON.parse disagree.
    if json.matches(&k).count() != 1 {
        return None;
    }
    let i = json.find(&k)? + k.len();
    let rest = json[i..].trim_start().strip_prefix(':')?.trim_start();
    if let Some(r) = rest.strip_prefix('"') {
        Some(&r[..r.find('"')?])
    } else {
        let end = rest.find(|c: char| c == ',' || c == '}' || c == '\n').unwrap_or(rest.len());
        Some(rest[..end].trim())
    }
}
fn sigs(json: &str) -> Vec<(String, String)> {
    let Some(start) = json.find("\"sigs\"") else { return vec![] };
    let body = &json[start..];
    let mut out = vec![];
    let mut rest = body;
    while let Some(i) = rest.find("\"pub\"") {
        let obj = &rest[i..];
        let end = obj.find('}').unwrap_or(obj.len());
        let o = &obj[..end];
        if let (Some(p), Some(s)) = (field(o, "pub"), field(o, "sig")) {
            out.push((p.to_string(), s.to_string()));
        }
        rest = &obj[end.min(obj.len())..];
        if end == obj.len() {
            break;
        }
    }
    out
}
fn render(genesis: &str, doc_sha: &str, salt: &str, c: &str, s: &[(String, String)]) -> String {
    let list: Vec<String> = s
        .iter()
        .map(|(p, sg)| {
            let handle = arr32(&unhex(p).unwrap_or_default()).map(|k| address_of(&k)).unwrap_or_default();
            format!("    {{\n      \"pub\": \"{p}\",\n      \"handle\": \"{handle}\",\n      \"sig\": \"{sg}\"\n    }}")
        })
        .collect();
    format!(
        "{{\n  \"v\": 1,\n  \"kind\": \"hlq-pact\",\n  \"genesis\": \"{genesis}\",\n  \"doc_sha256\": \"{doc_sha}\",\n  \"salt\": \"{salt}\",\n  \"commitment\": \"{c}\",\n  \"sigs\": [\n{}\n  ]\n}}\n",
        list.join(",\n")
    )
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(|s| s.as_str()) {
        Some("new") if a.len() == 3 => {
            let doc = read(&a[1]);
            let g = arr32(&unhex(&a[2]).unwrap_or_default()).unwrap_or_else(|| die("genesis must be 32 bytes hex"));
            let w = wallet_from_stdin();
            let mut salt = [0u8; 32];
            std::fs::File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut salt))
                .unwrap_or_else(|_| die("no randomness source"));
            let d = document_digest(&doc);
            let c = pact_commitment(&salt, &d);
            let sig = w.sign_pact(&g, &c);
            print!("{}", render(&hex(&g), &hex(&d), &hex(&salt), &hex(&c), &[(hex(&w.public_key()), hex(&sig))]));
        }
        Some("sign") if a.len() == 2 => {
            let j = String::from_utf8(read(&a[1])).unwrap_or_else(|_| die("package is not text"));
            let (g, d, salt, c) = (field(&j, "genesis"), field(&j, "doc_sha256"), field(&j, "salt"), field(&j, "commitment"));
            let (Some(g), Some(d), Some(salt), Some(c)) = (g, d, salt, c) else { die("not a pact package") };
            let (gb, cb) = (arr32(&unhex(g).unwrap_or_default()), arr32(&unhex(c).unwrap_or_default()));
            let (Some(gb), Some(cb)) = (gb, cb) else { die("malformed package") };
            let check = pact_commitment(
                &arr32(&unhex(salt).unwrap_or_default()).unwrap_or_else(|| die("bad salt")),
                &arr32(&unhex(d).unwrap_or_default()).unwrap_or_else(|| die("bad digest")),
            );
            if check != cb {
                die("the package's commitment does not match its salt and digest: refusing to sign");
            }
            let w = wallet_from_stdin();
            let me = hex(&w.public_key());
            let mut s = sigs(&j);
            if s.iter().any(|(p, _)| *p == me) {
                die("you already signed this pact");
            }
            s.push((me, hex(&w.sign_pact(&gb, &cb))));
            print!("{}", render(g, d, salt, c, &s));
        }
        Some("verify") if a.len() == 2 || a.len() == 3 => {
            let j = String::from_utf8(read(&a[1])).unwrap_or_else(|_| die("package is not text"));
            let (Some(g), Some(d), Some(salt), Some(c)) =
                (field(&j, "genesis"), field(&j, "doc_sha256"), field(&j, "salt"), field(&j, "commitment"))
            else {
                die("not a pact package")
            };
            let gb = arr32(&unhex(g).unwrap_or_default()).unwrap_or_else(|| die("bad genesis"));
            let cb = arr32(&unhex(c).unwrap_or_default()).unwrap_or_else(|| die("bad commitment"));
            let db = arr32(&unhex(d).unwrap_or_default()).unwrap_or_else(|| die("bad digest"));
            let sb = arr32(&unhex(salt).unwrap_or_default()).unwrap_or_else(|| die("bad salt"));
            let mut all = true;
            let ok_c = pact_commitment(&sb, &db) == cb;
            all &= ok_c;
            println!("{} commitment matches salt + document digest", if ok_c { "OK  " } else { "FAIL" });
            if a.len() == 3 {
                let ok_d = document_digest(&read(&a[2])) == db;
                all &= ok_d;
                println!("{} the document is THIS text, byte for byte", if ok_d { "OK  " } else { "FAIL" });
            }
            let s = sigs(&j);
            if s.is_empty() {
                all = false;
                println!("FAIL no signatures");
            }
            let mut raw = vec![];
            for (p, sg) in &s {
                let pk = arr32(&unhex(p).unwrap_or_default());
                let sig: Option<[u8; 64]> = unhex(sg).and_then(|v| v.try_into().ok());
                let ok = matches!((pk, sig), (Some(k), Some(x)) if Wallet::verify_pact(&k, &gb, &cb, &x).is_ok());
                all &= ok;
                if let Some(x) = sig {
                    raw.push(x);
                }
                let who = pk.map(|k| address_of(&k)).unwrap_or_else(|| "?".into());
                println!("{} signature of {who}", if ok { "OK  " } else { "FAIL" });
            }
            if !raw.is_empty() {
                println!("sealed key (mode B): {}", hex(&pact_sealed_key(&cb, &raw)));
            }
            std::process::exit(if all { 0 } else { 1 });
        }
        _ => die("usage: new <document> <genesis_hex> | sign <pact.hlqpact> | verify <pact.hlqpact> [document]   (24 words on stdin for new/sign)"),
    }
}
