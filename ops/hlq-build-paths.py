#!/usr/bin/env python3
"""hlq-build-paths — does a node binary or runtime wasm carry the build machine's paths?

Why (2026-09-17). build-node.sh remaps cargo/rustup/project paths to neutral /hlq-build/… so a binary
neither leaks the build account nor depends on where it was compiled. But substrate-wasm-builder compiles
the runtime in a separate cargo run and OVERWRITES RUSTFLAGS, so the runtime wasm never got the remap. The
runtime spec 3 live on-chain carries 96 `/home/<account>/…` strings. `strings` on the node binary shows
nothing, because the runtime sits inside it zstd-compressed — this tool decompresses it first.

Usage: hlq-build-paths.py <binary-or-wasm> [...]
For each file and each embedded runtime blob it prints the count of absolute home paths and of remapped
/hlq-build/ paths. Exit 0 = no home paths anywhere, 1 = leak found, 2 = could not read/decompress (a blob
we cannot open is NOT a clean blob).
"""
import re, subprocess, sys

MAGIC = bytes([0x52, 0xBC, 0x53, 0x76, 0x46, 0xDB, 0x8E, 0x05])  # sp-maybe-compressed-blob ZSTD prefix
HOME = re.compile(rb"/(?:home|Users)/[A-Za-z0-9._-]+/")
REMAP = re.compile(rb"/hlq-build/")


def scan(label, data):
    homes = HOME.findall(data)
    accounts = sorted({h.decode(errors="replace") for h in homes})
    print(f"{label}: home_paths={len(homes)} hlq_build={len(REMAP.findall(data))}"
          + (f" accounts={','.join(accounts)}" if accounts else ""))
    return len(homes)


def unzstd(buf):
    r = subprocess.run(["zstd", "-dc", "--no-check"], input=buf, capture_output=True)
    return r.stdout if r.stdout else None


def main(paths):
    if not paths:
        print(__doc__); return 2
    leaks, blind = 0, 0
    for p in paths:
        try:
            data = open(p, "rb").read()
        except OSError as e:
            print(f"{p}: UNREADABLE {e}"); blind += 1; continue
        leaks += scan(p, data)
        i, n = 0, 0
        while (j := data.find(MAGIC, i)) >= 0:
            i = j + 1
            out = unzstd(data[j + 8:j + 8 + 64_000_000])
            if out is None or not out.startswith(b"\0asm"):
                continue  # the prefix can occur by chance; only real wasm counts
            n += 1
            leaks += scan(f"{p} [embedded runtime #{n}, {len(out)} B]", out)
        if data.startswith(MAGIC) and n == 0:
            print(f"{p}: compressed blob that did not decompress to wasm"); blind += 1
    if blind:
        print("RESULT=UNKNOWN"); return 2
    print("RESULT=" + ("LEAK" if leaks else "CLEAN"))
    return 1 if leaks else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
