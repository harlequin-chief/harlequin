//! harlequin-heartbeat — the daily beat of a node bound to a mask (hosting reward E1, 2026-10-02).
//!
//! Reads the node's vote key (the file `harlequin-link-to-mask` created), asks the node's OWN RPC for the hash of a
//! recent block, signs `"hlq-heartbeat-v1" ‖ hot ‖ at (u32 LE) ‖ hash` and submits `Hosting::heartbeat` as an
//! UNSIGNED extrinsic to that same local RPC. The vote key never leaves the machine and needs no coins.
//! Run once a day (the installer adds a timer). A beat the chain refuses (already today, not bound yet) is reported
//! and exits 1; it is never retried in a loop.
//!
//! Usage: harlequin-heartbeat --key-file /opt/harlequin/data/vote.key [--rpc 127.0.0.1:9944]
use sp_core::{sr25519, Pair};
use std::io::{Read, Write};
use std::net::TcpStream;

const LABEL: &[u8; 16] = b"hlq-heartbeat-v1";
/// Runtime pallet index of Hosting and its call index (runtime/src/lib.rs: pallet_index(16), call_index(0)).
const PALLET: u8 = 16;
const CALL: u8 = 0;
/// How far behind the tip to point (the runtime accepts ≤ 600): far enough to be on every node's chain.
const LAG: u32 = 20;

fn die(msg: &str) -> ! {
    eprintln!("harlequin-heartbeat: {msg}");
    std::process::exit(2)
}

fn rpc(addr: &str, method: &str, params: &str) -> String {
    let body = format!("{{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"{method}\",\"params\":{params}}}");
    let mut s = TcpStream::connect(addr).unwrap_or_else(|e| die(&format!("cannot reach the node RPC at {addr}: {e}")));
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(20)));
    write!(s, "POST / HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        .unwrap_or_else(|e| die(&format!("rpc write: {e}")));
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
}

fn json_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let k = format!("\"{key}\":\"");
    let i = json.find(&k)? + k.len();
    Some(&json[i..i + json[i..].find('"')?])
}

/// SCALE compact encoding of a length.
fn compact(n: usize) -> Vec<u8> {
    match n {
        0..=63 => vec![(n as u8) << 2],
        64..=16383 => ((n as u16) << 2 | 1).to_le_bytes().to_vec(),
        _ => ((n as u32) << 2 | 2).to_le_bytes().to_vec(),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (mut key_file, mut addr) = (None::<String>, "127.0.0.1:9944".to_string());
    let mut it = a.iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--key-file" => key_file = it.next().cloned(),
            "--rpc" => addr = it.next().cloned().unwrap_or_else(|| die("--rpc needs host:port")),
            _ => die(&format!("unknown argument: {x}")), // never fall back to a default path
        }
    }
    let key_file = key_file.unwrap_or_else(|| die("usage: harlequin-heartbeat --key-file <vote.key> [--rpc 127.0.0.1:9944]"));
    let suri = std::fs::read_to_string(&key_file).unwrap_or_else(|e| die(&format!("cannot read {key_file}: {e}")));
    let pair = sr25519::Pair::from_string(suri.trim(), None).unwrap_or_else(|_| die("the vote key file is not a usable key"));
    let hot: [u8; 32] = pair.public().0;

    let head = rpc(&addr, "chain_getHeader", "[]");
    let tip = json_str(&head, "number").and_then(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok())
        .unwrap_or_else(|| die("the node did not return its head (is it synced?)"));
    if tip <= LAG {
        die("the chain is too young for a beat yet");
    }
    let at = tip - LAG;
    let hash_hex = rpc(&addr, "chain_getBlockHash", &format!("[{at}]"));
    let hash_hex = json_str(&hash_hex, "result").unwrap_or_else(|| die("no block hash from the node"));
    let hash: [u8; 32] = hex::decode(hash_hex.trim_start_matches("0x")).ok().and_then(|v| v.try_into().ok())
        .unwrap_or_else(|| die("bad block hash"));

    let mut msg = Vec::with_capacity(84);
    msg.extend_from_slice(LABEL);
    msg.extend_from_slice(&hot);
    msg.extend_from_slice(&at.to_le_bytes());
    msg.extend_from_slice(&hash);
    let sig = pair.sign(&msg).0;

    // Unsigned (bare) extrinsic, format v4: compact(len) ‖ 0x04 ‖ call(pallet, call, at, hash, hot, sig).
    let mut body = vec![0x04u8, PALLET, CALL];
    body.extend_from_slice(&at.to_le_bytes());
    body.extend_from_slice(&hash);
    body.extend_from_slice(&hot);
    body.extend_from_slice(&sig);
    let mut xt = compact(body.len());
    xt.extend_from_slice(&body);
    let res = rpc(&addr, "author_submitExtrinsic", &format!("[\"0x{}\"]", hex::encode(&xt)));
    if let Some(h) = json_str(&res, "result") {
        println!("beat submitted for block #{at} (tx {h})");
    } else {
        eprintln!("the node refused the beat: {}", res.chars().take(300).collect::<String>());
        eprintln!("(already beat today, the node is not linked to a mask yet, or the chain has no Hosting pallet)");
        std::process::exit(1);
    }
}
