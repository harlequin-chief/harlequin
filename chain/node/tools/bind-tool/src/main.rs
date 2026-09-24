//! bind-tool — the NODE side of binding a node to its owner's mask (station 5).
//!
//! ## Why this tool exists, and why it deliberately does so little
//!
//! Binding is one on-chain call, `Reputation::set_vote_key { sk_pub, pop_sig }`, signed by the MASK.
//! A mask lives in its owner's browser and nowhere else: that is the whole promise of the project, so
//! no tool of ours — least of all one that runs on a server — may ever see, ask for, or touch an
//! account secret. This tool therefore does only the half that belongs to the machine:
//!
//!   1. makes sure the node HAS a vote key (creates one, 0600, if it does not),
//!   2. proves possession of that key for ONE named mask, and
//!   3. prints the two values that proof consists of — both PUBLIC.
//!
//! The owner pastes those two values into their own panel, where their mask signs the call and sends
//! it. The secret never crosses the wire; the node's key never leaves the node.
//!
//! ## What the printed values are, exactly
//!
//! * `sk_pub`   — the node's vote PUBLIC key. Publishing it reveals nothing; it is what the chain
//!                will map to the mask.
//! * `pop_sig`  — the node's vote key signing `"hlq-node-bind-v1" ‖ <mask account, 32 bytes>`. It
//!                proves the node holds the key AND ties that proof to THAT mask: the same signature
//!                is worthless for any other account, so it can be pasted, mailed or read aloud.
//!
//! ## Re-running is expected, not exceptional
//!
//! Machines die. `--rotate` mints a fresh vote key so the owner can bind a replacement node with the
//! same mask; the chain treats a second `set_vote_key` from the same account as a rotation and
//! releases the old key. Nobody gets locked out of their own node for losing a disk.
//!
//! Binding records WHO did the work. It is not payment and it does not create any: service is paid
//! only for authorship and finality votes inside the committee, and the committee is entered through
//! earned reputation.

use clap::Parser;
use sp_core::crypto::{AccountId32, Ss58Codec};
use sp_core::{sr25519, Pair};
use std::io::Write;
use std::path::Path;

/// Must match `pallet_reputation::BIND_LABEL` byte for byte — the chain verifies this exact prefix.
const BIND_LABEL: &[u8] = b"hlq-node-bind-v1";

#[derive(Parser)]
#[command(
    about = "Prints the two PUBLIC values your mask needs in order to claim this node as yours.",
    after_help = "Safety: rotating always keeps a dated 0600 backup of the previous key, and refuses \
to run while a node visible from here is using it. That check cannot see a node running inside a \
container when this is run from the host — the backup is what covers that case."
)]
struct Args {
    /// Your mask's address (the one you created in the browser).
    #[arg(long)]
    mask: String,
    /// Where the node's vote key lives. Created with 0600 permissions if missing.
    #[arg(long, default_value = "/opt/harlequin/session.secret")]
    key_file: String,
    /// Mint a NEW vote key even if one exists (use when replacing a machine).
    #[arg(long, default_value_t = false)]
    rotate: bool,
    /// Rotate even though a node is RUNNING with this key. Only for someone who knows the node
    /// will be restarted and re-claimed: until then its votes count for nobody.
    #[arg(long, default_value_t = false)]
    force: bool,
}

/// Is a live process already signing with this key file? Refusing to rotate under one is the
/// difference between a harmless command on a newcomer's box and a destroyed validator on ours:
/// the default key path is the SAME on both, and the file is truncated on write.
///
/// KNOWN LIMIT, stated rather than hidden: this sees only the processes THIS side can see. Run from
/// a host against a key file inside a container, the node is invisible here and the guard will not
/// fire. The backup still covers that case, which is why the backup — not the guard — is the part
/// that must never be skipped.
fn node_running_with(key_file: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else { return false };
    for e in entries.flatten() {
        let p = e.path().join("cmdline");
        let Ok(raw) = std::fs::read(&p) else { continue };
        let args: Vec<String> =
            raw.split(|b| *b == 0).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let hit = if let Some(v) = a.strip_prefix("--vote-as-file=") {
                v == key_file
            } else if a == "--vote-as-file" {
                it.next().map(|v| v == key_file).unwrap_or(false)
            } else {
                false
            };
            if hit {
                return true;
            }
        }
    }
    false
}

/// Keep the previous key next to the new one. Rotating used to be irreversible: the file is opened
/// with truncate, so a mistyped command on the wrong machine erased a validator's identity with no
/// copy anywhere. A dated 0600 backup costs nothing and removes "no way back" from the failure list.
fn backup_existing(path: &Path) -> std::io::Result<Option<std::path::PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = path.with_file_name(format!(
        "{}.bak-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        stamp
    ));
    let contents = std::fs::read_to_string(path)?;
    write_secret(&dest, &contents)?;
    Ok(Some(dest))
}

fn main() {
    let args = Args::parse();

    let account = match AccountId32::from_ss58check(args.mask.trim()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("  x that does not look like a mask address: {e}");
            eprintln!("    it is the address you copied when you created your mask in the browser.");
            std::process::exit(2);
        },
    };

    let path = Path::new(&args.key_file);
    let existed = path.exists();

    // Everything that can destroy something is decided BEFORE a single byte is written.
    if existed && args.rotate {
        let live = node_running_with(&args.key_file);
        if live && !args.force {
            eprintln!();
            eprintln!("  x REFUSING: a node is RUNNING right now with {}.", args.key_file);
            eprintln!("    Rotating would replace the key that node is signing with, and this file is");
            eprintln!("    overwritten in place. On a node that already serves in the committee that");
            eprintln!("    means it stops being credited until it is restarted AND re-claimed.");
            eprintln!();
            eprintln!("    If this really is a machine you are replacing: stop the node, run this again.");
            eprintln!("    If you know exactly what you are doing: re-run with --force.");
            eprintln!();
            eprintln!("    (This check only sees processes visible from here. Run from outside a");
            eprintln!("     container against a key file inside it, it cannot warn you — the backup can.)");
            eprintln!();
            std::process::exit(3);
        }
        println!();
        println!("  ! about to replace the vote key in {}.", args.key_file);
        println!("    ORDER MATTERS: restart the node with the new key FIRST, send the claim after.");
        if live {
            println!("    (a node IS running with this key — you passed --force, so it will go mute");
            println!("     until you restart it and its new claim is accepted.)");
        }
        match backup_existing(path) {
            Ok(Some(dest)) => println!("    a copy of the old key is kept at {}", dest.display()),
            Ok(None) => {},
            Err(e) => {
                eprintln!("  x cannot back up the existing key ({e}) — refusing to overwrite it.");
                std::process::exit(2);
            },
        }
    }

    let pair = if existed && !args.rotate {
        let suri = match std::fs::read_to_string(path) {
            Ok(s) => s.trim().to_string(),
            Err(e) => {
                eprintln!("  x cannot read {}: {e}", args.key_file);
                std::process::exit(2);
            },
        };
        match sr25519::Pair::from_string(&suri, None) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("  x {} does not contain a usable key: {e:?}", args.key_file);
                eprintln!("    re-run with --rotate to mint a fresh one.");
                std::process::exit(2);
            },
        }
    } else {
        let (pair, seed) = sr25519::Pair::generate();
        if let Err(e) = write_secret(path, &format!("0x{}\n", hex::encode(seed))) {
            eprintln!("  x cannot write {}: {e}", args.key_file);
            std::process::exit(2);
        }
        pair
    };

    let mut msg = BIND_LABEL.to_vec();
    msg.extend_from_slice(account.as_ref());
    let pop = pair.sign(&msg);

    println!();
    println!("  ── claiming this node for your mask ──────────────────────");
    println!();
    if !existed {
        println!("  * created this node's vote key at {} (only root can read it).", args.key_file);
    } else if args.rotate {
        println!("  * minted a NEW vote key at {} (the previous one is backed up beside it).",
            args.key_file);
    } else {
        println!("  * this node already had a vote key; reusing it.");
    }
    println!();
    println!("  BOTH VALUES BELOW ARE PUBLIC. Paste them anywhere without fear: they are useless");
    println!("  to anyone else, because the second one only works for YOUR mask and no other.");
    println!("  Nothing secret is printed here, and this tool never asks for your seed phrase.");
    println!();
    println!("  mask       {}", account.to_ss58check());
    println!("  vote key   0x{}", hex::encode(pair.public().0));
    println!("  proof      0x{}", hex::encode(pop.0));
    println!();
    println!("  Next: open your panel, paste those two values, and your mask signs the claim there.");
    println!("  For a mask that has never spent anything, that signature costs no fee.");
    println!();
    println!("  Then make the node USE this key, if it is not already:");
    println!("    add   --vote-as-file {}   to the service and restart it.", args.key_file);
    println!();
    println!("  This records that the work is yours. It is not payment: service is paid inside the");
    println!("  committee, and the committee is entered through reputation you earn.");
    println!();
}

/// Write the secret with 0600 from the start — never world-readable, not even for an instant.
fn write_secret(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(contents.as_bytes())
}
