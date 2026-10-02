//! Recovery phrase for the wallet (#664, 2026-06-25).
//!
//! The mask/wallet is seeded by 32 bytes of entropy ([`crate::Wallet::from_entropy`]). A human cannot
//! safely write down 32 raw bytes, so the onboarding ("recital de bienvenida") shows the entropy as a
//! **BIP-39 24-word mnemonic** — the recovery phrase. Write it down → you can always rebuild the same
//! mask/wallet, with no server and no custodian.
//!
//!   entropy(32B)  →  24 words   (`entropy_to_phrase`)
//!   24 words      →  entropy(32B)   (`phrase_to_entropy`, validates the BIP-39 checksum)
//!
//! BIP-39 English wordlist, standard checksum. 32 bytes of entropy = 24 words (256 bits). Vetted crate
//! (`bip39`, pinned + audited). The phrase IS the secret — treat it like the key.

use bip39::Mnemonic;

#[derive(Debug, PartialEq, Eq)]
pub enum MnemonicError {
    /// The phrase is not valid BIP-39 (bad word, wrong length, or failed checksum).
    Invalid,
    /// The phrase decoded, but not to 32 bytes of entropy (not a 24-word Harlequin seed).
    WrongLength,
}

/// 32-byte entropy → 24-word BIP-39 English recovery phrase.
pub fn entropy_to_phrase(entropy: &[u8; 32]) -> String {
    // from_entropy only fails on invalid length; 32 is always valid → unwrap is safe here.
    Mnemonic::from_entropy(entropy)
        .expect("32 bytes is a valid BIP-39 entropy length")
        .to_string()
}

/// 24-word BIP-39 phrase → 32-byte entropy. Validates the checksum; rejects anything not a 32-byte seed.
pub fn phrase_to_entropy(phrase: &str) -> Result<[u8; 32], MnemonicError> {
    let m = Mnemonic::parse_normalized(phrase).map_err(|_| MnemonicError::Invalid)?;
    let (entropy, len) = m.to_entropy_array();
    if len != 32 {
        return Err(MnemonicError::WrongLength);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&entropy[..32]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Wallet;

    #[test]
    fn roundtrip_entropy_phrase() {
        let entropy = [0x42u8; 32];
        let phrase = entropy_to_phrase(&entropy);
        assert_eq!(phrase.split_whitespace().count(), 24, "32 bytes → 24 words");
        assert_eq!(phrase_to_entropy(&phrase).unwrap(), entropy);
    }

    #[test]
    fn phrase_recovers_same_wallet() {
        let entropy = [0x13u8; 32];
        let w1 = Wallet::from_entropy(&entropy).unwrap();
        let phrase = entropy_to_phrase(&entropy);
        // user writes down `phrase`, later recovers:
        let recovered = phrase_to_entropy(&phrase).unwrap();
        let w2 = Wallet::from_entropy(&recovered).unwrap();
        assert_eq!(w1.address(), w2.address(), "the phrase rebuilds the exact same mask/wallet");
    }

    #[test]
    fn distinct_entropy_distinct_phrase() {
        assert_ne!(entropy_to_phrase(&[1u8; 32]), entropy_to_phrase(&[2u8; 32]));
    }

    #[test]
    fn bad_checksum_rejected() {
        // valid words but tampered last word → checksum fails.
        let mut words: Vec<&str> = "abandon abandon abandon abandon abandon abandon abandon abandon \
            abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
            abandon abandon abandon abandon abandon abandon".split_whitespace().collect();
        words[23] = "zoo"; // breaks the checksum
        let bad = words.join(" ");
        assert_eq!(phrase_to_entropy(&bad), Err(MnemonicError::Invalid));
    }

    #[test]
    fn gibberish_rejected() {
        assert_eq!(phrase_to_entropy("not a real mnemonic phrase at all"), Err(MnemonicError::Invalid));
    }

    #[test]
    fn short_phrase_wrong_length() {
        // A valid 12-word phrase parses but is 16 bytes, not a 32-byte Harlequin seed.
        let twelve = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        assert_eq!(phrase_to_entropy(twelve), Err(MnemonicError::WrongLength));
    }
}
