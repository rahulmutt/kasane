#!/usr/bin/env python3
"""Regenerate minimal.mobi and minimal-drm.mobi.
Run from anywhere: python3 tests/fixtures/mobi/make_minimal_mobi.py"""
import base64, pathlib, struct

HERE = pathlib.Path(__file__).parent
# 1x1 red PNG (same bytes as the EPUB rich fixture)
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842"
    "iQAAAABJRU5ErkJggg=="
)

HTML = (
    '<html><head></head><body>'
    '<h1>Chapter One</h1>'
    '<p>Alpha <i>beta</i> <b>gamma</b>, see <a filepos=FILEPOS0>Chapter Two</a>.</p>'
    '<ul><li>alpha</li><li>beta<ul><li>beta-one</li></ul></li></ul>'
    '<p><img recindex="00001" alt="The red dot"></p>'
    '<mbp:pagebreak/>'
    '<h1>Chapter Two</h1>'
    '<p>Delta text.</p>'
    '</body></html>'
)


def build_text():
    # Zero-padded placeholder keeps offsets stable across the substitution.
    html = HTML.replace("FILEPOS0", "0" * 10)
    target = html.index("<h1>Chapter Two")
    html = html.replace("filepos=" + "0" * 10, "filepos=%010d" % target)
    return html.encode("utf-8")


def record0(text_len, text_recs, first_image_rec, encryption=0):
    palmdoc = struct.pack(">HHIHHHH", 1, 0, text_len, text_recs, 4096, encryption, 0)
    title = b"Minimal Mobi"
    mobi_hlen = 232                      # header spans record0 bytes 16..248
    full_name_off = 16 + mobi_hlen       # title appended right after the header
    m = bytearray(mobi_hlen)
    m[0:4] = b"MOBI"
    struct.pack_into(">I", m, 4, mobi_hlen)       # header length   (rec0 @20)
    struct.pack_into(">I", m, 8, 2)               # type: book      (rec0 @24)
    struct.pack_into(">I", m, 12, 65001)          # encoding        (rec0 @28)
    struct.pack_into(">I", m, 16, 1)              # uid             (rec0 @32)
    struct.pack_into(">I", m, 20, 6)              # version         (rec0 @36)
    struct.pack_into(">I", m, 68, full_name_off)  # full name off   (rec0 @0x54)
    struct.pack_into(">I", m, 72, len(title))     # full name len   (rec0 @0x58)
    struct.pack_into(">I", m, 92, first_image_rec)  # first image   (rec0 @0x6C)
    struct.pack_into(">H", m, 226, 0)             # extra_flags     (rec0 @0xF2)
    return palmdoc + bytes(m) + title + b"\x00\x00"


def palmdb(records):
    n = len(records)
    hdr = bytearray(78)
    hdr[0:12] = b"minimal-mobi"
    hdr[60:64] = b"BOOK"
    hdr[64:68] = b"MOBI"
    struct.pack_into(">H", hdr, 76, n)
    pos = 78 + 8 * n + 2
    table = b""
    for i, r in enumerate(records):
        table += struct.pack(">IB", pos, 0) + (2 * i).to_bytes(3, "big")
        pos += len(r)
    return bytes(hdr) + table + b"\x00\x00" + b"".join(records)


def build(encryption):
    text = build_text()
    chunks = [text[i:i + 4096] for i in range(0, len(text), 4096)]
    # records: 0=headers, 1..n=text, n+1=image  -> first_image_rec = 1+len(chunks)
    rec0 = record0(len(text), len(chunks), 1 + len(chunks), encryption)
    return palmdb([rec0] + chunks + [PNG])


(HERE / "minimal.mobi").write_bytes(build(encryption=0))
(HERE / "minimal-drm.mobi").write_bytes(build(encryption=2))
print("wrote", HERE / "minimal.mobi", "and minimal-drm.mobi")
