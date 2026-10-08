"""hlq_scale — minimal SCALE decoder driven by runtime metadata V14. Stdlib only (like the mediator).

Why (7-oct-2026, #1531): the wallet's "Activity" needs the chain's Tokens.Transferred events, and
System.Events is a Vec<EventRecord> whose records have variable length: skipping the ones we do not want
requires decoding them all. Rather than hand-writing the layout of every pallet's events (which would
break on the next runtime upgrade), we read the layout from the runtime's own metadata.

Read-only: decodes bytes the node gives us; never builds or signs anything.
"""

# twox128("System") ++ twox128("Events") — a fixed constant of every Substrate chain (stdlib has no xxhash).
SYSTEM_EVENTS_KEY = "0x26aa394eea5630e07c48ae0c9558cef780d41e5e16056765bc8461851072c9d7"


class R:
    """Byte reader."""
    __slots__ = ("b", "i")

    def __init__(self, b, i=0):
        self.b, self.i = b, i

    def take(self, n):
        if self.i + n > len(self.b):
            raise ValueError("scale: out of bytes")
        v = self.b[self.i:self.i + n]
        self.i += n
        return v

    def u8(self):
        return self.take(1)[0]

    def uint(self, n):
        return int.from_bytes(self.take(n), "little")

    def compact(self):
        b0 = self.u8()
        m = b0 & 3
        if m == 0:
            return b0 >> 2
        if m == 1:
            return (b0 | (self.u8() << 8)) >> 2
        if m == 2:
            return int.from_bytes(bytes([b0]) + self.take(3), "little") >> 2
        return int.from_bytes(self.take((b0 >> 2) + 4), "little")

    def string(self):
        return self.take(self.compact()).decode("utf-8", "replace")

    def opt(self, f):
        return f() if self.u8() else None

    def vec(self, f):
        return [f() for _ in range(self.compact())]


PRIM_SIZE = {3: 1, 4: 2, 5: 4, 6: 8, 7: 16, 8: 32, 9: 1, 10: 2, 11: 4, 12: 8, 13: 16, 14: 32}


class Metadata:
    """Parses RuntimeMetadataPrefixed V14 enough to decode storage values and events."""

    def __init__(self, raw: bytes):
        r = R(raw)
        if r.take(4) != b"meta":
            raise ValueError("metadata: bad magic")
        ver = r.u8()
        if ver != 14:
            raise ValueError(f"metadata: version {ver} not supported (V14 only)")
        self.types = {}
        for _ in range(r.compact()):
            tid = r.compact()
            self.types[tid] = self._type(r)
        self.pallets = {}
        for _ in range(r.compact()):
            p = self._pallet(r)
            self.pallets[p["name"]] = p
        # (extrinsic metadata and the runtime type follow; not needed)
        self._cache = {}

    # ── registry parsing ─────────────────────────────────────────────────────────────────────────
    def _field(self, r):
        name = r.opt(r.string)
        ty = r.compact()
        r.opt(r.string)          # type_name
        r.vec(r.string)          # docs
        return (name, ty)

    def _type(self, r):
        path = r.vec(r.string)
        r.vec(lambda: (r.string(), r.opt(r.compact)))   # type params
        k = r.u8()
        if k == 0:
            d = ("composite", r.vec(lambda: self._field(r)))
        elif k == 1:
            vs = {}
            for _ in range(r.compact()):
                name = r.string()
                fields = r.vec(lambda: self._field(r))
                idx = r.u8()
                r.vec(r.string)
                vs[idx] = (name, fields)
            d = ("variant", vs)
        elif k == 2:
            d = ("seq", r.compact())
        elif k == 3:
            n = r.uint(4)
            d = ("array", n, r.compact())
        elif k == 4:
            d = ("tuple", r.vec(r.compact))
        elif k == 5:
            d = ("prim", r.u8())
        elif k == 6:
            d = ("compact", r.compact())
        elif k == 7:
            d = ("bits", r.compact(), r.compact())
        else:
            raise ValueError(f"metadata: unknown typedef {k}")
        r.vec(r.string)          # docs
        return {"path": path, "def": d}

    def _pallet(self, r):
        name = r.string()
        storage = None
        if r.u8():
            prefix = r.string()
            entries = {}
            for _ in range(r.compact()):
                en = r.string()
                r.u8()           # modifier
                if r.u8() == 0:
                    ety = ("plain", r.compact())
                else:
                    hashers = r.vec(r.u8)
                    key = r.compact()
                    val = r.compact()
                    ety = ("map", hashers, key, val)
                r.take(r.compact())   # default
                r.vec(r.string)
                entries[en] = ety
            storage = {"prefix": prefix, "entries": entries}
        calls = r.opt(r.compact)
        event = r.opt(r.compact)
        for _ in range(r.compact()):   # constants
            r.string(); r.compact(); r.take(r.compact()); r.vec(r.string)
        error = r.opt(r.compact)
        index = r.u8()
        return {"name": name, "storage": storage, "calls": calls, "event": event, "error": error, "index": index}

    # ── decoding ─────────────────────────────────────────────────────────────────────────────────
    def decode(self, tid, r):
        t = self.types[tid]
        d = t["def"]
        k = d[0]
        if k == "prim":
            p = d[1]
            if p == 0:
                return bool(r.u8())
            if p == 1:
                return chr(r.uint(4))
            if p == 2:
                return r.string()
            n = PRIM_SIZE[p]
            v = r.uint(n)
            if p >= 9 and v >= 1 << (8 * n - 1):
                v -= 1 << (8 * n)
            return v
        if k == "compact":
            return r.compact()
        if k == "array":
            n, inner = d[1], d[2]
            it = self.types[inner]["def"]
            if it == ("prim", 3):
                return "0x" + r.take(n).hex()
            return [self.decode(inner, r) for _ in range(n)]
        if k == "seq":
            inner = d[1]
            n = r.compact()
            if self.types[inner]["def"] == ("prim", 3):
                return "0x" + r.take(n).hex()
            return [self.decode(inner, r) for _ in range(n)]
        if k == "tuple":
            return [self.decode(x, r) for x in d[1]]
        if k == "composite":
            fs = d[1]
            if len(fs) == 1 and fs[0][0] is None:
                return self.decode(fs[0][1], r)         # newtype (AccountId32([u8;32]) → hex)
            if fs and all(f[0] is None for f in fs):
                return [self.decode(f[1], r) for f in fs]
            return {f[0]: self.decode(f[1], r) for f in fs}
        if k == "variant":
            idx = r.u8()
            if idx not in d[1]:
                raise ValueError(f"scale: variant index {idx} unknown in type {tid} {t['path']}")
            name, fs = d[1][idx]
            if not fs:
                return name
            if all(f[0] is None for f in fs):
                vals = [self.decode(f[1], r) for f in fs]
                return {name: vals[0] if len(vals) == 1 else vals}
            return {name: {f[0]: self.decode(f[1], r) for f in fs}}
        if k == "bits":
            n = r.compact()
            store = self.types[d[1]]["def"]
            size = PRIM_SIZE.get(store[1], 1) if store[0] == "prim" else 1
            r.take(((n + 8 * size - 1) // (8 * size)) * size)
            return f"<{n} bits>"
        raise ValueError(f"scale: cannot decode {k}")

    def storage_type(self, pallet, entry):
        e = self.pallets[pallet]["storage"]["entries"][entry]
        return e[1] if e[0] == "plain" else e[3]

    def events(self, raw: bytes):
        """System.Events → [(pallet_name, event_name, fields, extrinsic_index|None)]."""
        tid = self.storage_type("System", "Events")
        out = []
        r = R(raw)
        n = r.compact()
        rec = self.types[tid]["def"][1]                    # Vec<EventRecord> → EventRecord type id
        fields = dict(self.types[rec]["def"][1])          # phase, event, topics
        for _ in range(n):
            phase = self.decode(fields["phase"], r)
            ev = self.decode(fields["event"], r)          # {Pallet: {Event: {...}}} (outer RuntimeEvent)
            self.decode(fields["topics"], r)
            (pallet, inner), = ev.items()
            if isinstance(inner, dict):
                (name, body), = inner.items()
            else:
                name, body = inner, None
            xi = phase.get("ApplyExtrinsic") if isinstance(phase, dict) else None
            out.append((pallet, name, body, xi))
        if r.i != len(raw):
            raise ValueError(f"scale: {len(raw) - r.i} trailing bytes in System.Events")
        return out
