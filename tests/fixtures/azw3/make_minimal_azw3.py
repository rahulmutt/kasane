#!/usr/bin/env python3
"""Regenerate minimal.azw3 and lying-skel.azw3.
Run from anywhere: python3 tests/fixtures/azw3/make_minimal_azw3.py"""
import base64, pathlib, struct

HERE = pathlib.Path(__file__).parent
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842"
    "iQAAAABJRU5ErkJggg=="
)

SKELS = [
    '<html><head><title>p0</title></head><body></body></html>',
    '<html><head><title>p1</title></head><body></body></html>',
]
FRAGS = [
    '<h1>Part One</h1>'
    '<p>Alpha, see <a href="kindle:pos:fid:0001:off:0000000000">Part Two</a>.</p>'
    '<table><thead><tr><th>Name</th><th>Value</th></tr></thead>'
    '<tr><td>a</td><td>1</td></tr></table>',
    '<h1>Part Two</h1>'
    '<p><img src="kindle:embed:0001?mime=image/png" alt="The red dot"></p>'
    '<pre><code class="language-rust">fn main() {}</code></pre>',
]


def build_raw():
    """Deconstructed KF8 text stream: [skel0][frag0][skel1][frag1]."""
    raw = b""
    skel_entries, frag_entries = [], []
    for sk, fr in zip(SKELS, FRAGS):
        sk_b, fr_b = sk.encode(), fr.encode()
        skelpos = len(raw)
        insert_local = sk_b.index(b"</body>")
        raw += sk_b + fr_b
        skel_entries.append((1, skelpos, len(sk_b)))               # frag_count, start, len
        frag_entries.append((skelpos + insert_local, len(raw) - len(fr_b), len(fr_b)))
    return raw, skel_entries, frag_entries


def fwd_varint(v):
    bs = []
    while True:
        bs.insert(0, v & 0x7F)
        v >>= 7
        if v == 0:
            break
    bs[-1] |= 0x80
    return bytes(bs)


def indx_pair(tagx_entries, entries):
    """One index = (header record with TAGX, one data record). `entries`:
    list of (name_bytes, control_byte, [values])."""
    hlen = 0x30
    hdr = bytearray(hlen)
    hdr[0:4] = b"INDX"
    struct.pack_into(">I", hdr, 4, hlen)
    struct.pack_into(">I", hdr, 0x18, 1)              # one data record
    struct.pack_into(">I", hdr, 0x1C, 65001)
    struct.pack_into(">I", hdr, 0x24, len(entries))
    tagx = b"TAGX" + struct.pack(">II", 12 + 4 * len(tagx_entries), 1)
    tagx += b"".join(bytes(e) for e in tagx_entries)

    dh = 0x20
    blob, offs = b"", []
    for name, ctrl, values in entries:
        offs.append(dh + len(blob))
        blob += bytes([len(name)]) + name + bytes([ctrl])
        blob += b"".join(fwd_varint(v) for v in values)
    data = bytearray(dh)
    data[0:4] = b"INDX"
    struct.pack_into(">I", data, 4, dh)
    struct.pack_into(">I", data, 0x14, dh + len(blob))  # IDXT start
    struct.pack_into(">I", data, 0x18, len(entries))
    idxt = b"IDXT" + b"".join(struct.pack(">H", o) for o in offs)
    return [bytes(hdr) + tagx, bytes(data) + blob + idxt]


SKEL_TAGX = [(1, 1, 0x01, 0), (6, 2, 0x02, 0), (0, 0, 0, 1)]
FRAG_TAGX = [(2, 1, 0x01, 0), (3, 1, 0x02, 0), (4, 1, 0x04, 0), (6, 2, 0x08, 0), (0, 0, 0, 1)]


def record0(text_len, text_recs, frag_idx, skel_idx, first_image_rec):
    palmdoc = struct.pack(">HHIHHHH", 1, 0, text_len, text_recs, 4096, 0, 0)
    title = b"KF8 Minimal"
    mobi_hlen = 248                     # header spans record0 bytes 16..264
    full_name_off = 16 + mobi_hlen
    m = bytearray(mobi_hlen)
    m[0:4] = b"MOBI"
    struct.pack_into(">I", m, 4, mobi_hlen)         # header length  (rec0 @20)
    struct.pack_into(">I", m, 8, 2)                 # type: book     (rec0 @24)
    struct.pack_into(">I", m, 12, 65001)            # encoding       (rec0 @28)
    struct.pack_into(">I", m, 16, 1)                # uid            (rec0 @32)
    struct.pack_into(">I", m, 20, 8)                # VERSION 8      (rec0 @36)
    struct.pack_into(">I", m, 68, full_name_off)    # full name off  (rec0 @0x54)
    struct.pack_into(">I", m, 72, len(title))       # full name len  (rec0 @0x58)
    struct.pack_into(">I", m, 92, first_image_rec)  # first image    (rec0 @0x6C)
    struct.pack_into(">H", m, 226, 0)               # extra_flags    (rec0 @0xF2)
    struct.pack_into(">I", m, 232, frag_idx)        # frag/div index (rec0 @0xF8)
    struct.pack_into(">I", m, 236, skel_idx)        # skel index     (rec0 @0xFC)
    return palmdoc + bytes(m) + title + b"\x00\x00"


def palmdb(records):
    n = len(records)
    hdr = bytearray(78)
    hdr[0:12] = b"minimal-azw3"
    hdr[60:64] = b"BOOK"
    hdr[64:68] = b"MOBI"
    struct.pack_into(">H", hdr, 76, n)
    pos = 78 + 8 * n + 2
    table = b""
    for i, r in enumerate(records):
        table += struct.pack(">IB", pos, 0) + (2 * i).to_bytes(3, "big")
        pos += len(r)
    return bytes(hdr) + table + b"\x00\x00" + b"".join(records)


def build(lying=False):
    raw, skel_entries, frag_entries = build_raw()
    if lying:
        fc, st, _ = skel_entries[0]
        skel_entries[0] = (fc, st, 0x4FFFF)  # length far past the stream end
    chunks = [raw[i:i + 4096] for i in range(0, len(raw), 4096)]
    skel_idx = 1 + len(chunks)           # records: 0, text..., skel pair, frag pair, png
    frag_idx = skel_idx + 2
    first_image = frag_idx + 2
    rec0 = record0(len(raw), len(chunks), frag_idx, skel_idx, first_image)
    skel_recs = indx_pair(
        SKEL_TAGX,
        [(b"SKEL%07d" % i, 0x03, [fc, st, ln]) for i, (fc, st, ln) in enumerate(skel_entries)],
    )
    frag_recs = indx_pair(
        FRAG_TAGX,
        [(str(ip).encode(), 0x0F, [0, i, i, st, ln])
         for i, (ip, st, ln) in enumerate(frag_entries)],
    )
    return palmdb([rec0] + chunks + skel_recs + frag_recs + [PNG])


(HERE / "minimal.azw3").write_bytes(build(lying=False))
(HERE / "lying-skel.azw3").write_bytes(build(lying=True))
print("wrote", HERE / "minimal.azw3", "and lying-skel.azw3")
