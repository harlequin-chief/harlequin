// Sign a villa login challenge (#637, contract 2026-07-21) with a fixed test mask. Prints pubkey/sig
// hex for the E2E against the mediator verifier. Args: <nonce_hex 32B> [genesis_hex 32B] [issued_at].
use wallet_core::Wallet;

fn main() {
    let w = Wallet::from_entropy(&[42u8; 32]).unwrap();
    let nonce = arg_bytes32(1, &"11".repeat(32));
    let genesis = arg_bytes32(2, "a1ab5e0b11ccadccb1f8f90692f4c588190b421532ab0cee9f56ad615da1fed6");
    let issued_at: u64 = std::env::args().nth(3).map(|s| s.parse().unwrap()).unwrap_or(1_753_000_000);
    let sig = w.sign_villa_login(&nonce, &genesis, issued_at);
    println!("{}", hex(&w.public_key())); // line 1 = pubkey
    println!("{}", hex(&sig)); // line 2 = sig
    println!("{}", hex(&wallet_core::villa_login_message(&nonce, &genesis, issued_at))); // line 3 = msg
}

fn arg_bytes32(n: usize, default: &str) -> [u8; 32] {
    let h = std::env::args().nth(n).unwrap_or_else(|| default.to_string());
    let v: Vec<u8> = (0..h.len() / 2)
        .map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap())
        .collect();
    v.try_into().expect("need 32 bytes of hex")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}
