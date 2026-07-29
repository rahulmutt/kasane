#!/usr/bin/env python3
"""Generate fuzz/seeds/epub/deep-nesting.epub.

A minimal EPUB whose single chapter nests <em> 5000 deep. Before the depth
bounds landed (design spec 2026-07-29 SS2.2), converting this aborted the
process with a stack overflow in the core's and writer's recursive inline
walks. It is a seed rather than an artifact because the bug was found by
designing the property tier, not by libFuzzer.
"""
import pathlib
import zipfile

DEPTH = 5000
ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = ROOT / "fuzz" / "seeds" / "epub" / "deep-nesting.epub"

CONTAINER = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf"
    media-type="application/oebps-package+xml"/></rootfiles>
</container>
"""

OPF = """<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Deep Nesting</dc:title><dc:identifier id="id">deep</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="c1"/></spine>
</package>
"""

CHAPTER = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<html xmlns="http://www.w3.org/1999/xhtml"><body>'
    "<h1>Deep</h1><p>" + "<em>" * DEPTH + "bottom" + "</em>" * DEPTH + "</p>"
    "</body></html>"
)

OUT.parent.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED) as z:
    # mimetype must be first and stored, per EPUB OCF.
    z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip",
               compress_type=zipfile.ZIP_STORED)
    z.writestr("META-INF/container.xml", CONTAINER)
    z.writestr("OEBPS/content.opf", OPF)
    # Stored, not deflated: 5000 repeated "<em>" opening/closing tags is so
    # repetitive that DEFLATE compresses it past 200:1, which is exactly the
    # shape crate::guard::check_expansion's MAX_RATIO zip-bomb guard exists to
    # reject -- read_entry_bytes would return Err(ParseError::Bomb) and the
    # chapter would be silently skipped before it ever reached the depth-bound
    # code this seed exists to exercise. Storing it uncompressed keeps the
    # ratio at 1:1 without changing the nesting depth at all.
    z.writestr(zipfile.ZipInfo("OEBPS/c1.xhtml"), CHAPTER,
               compress_type=zipfile.ZIP_STORED)

print(f"wrote {OUT} ({OUT.stat().st_size} bytes)")
