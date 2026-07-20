#!/usr/bin/env python3
"""Regenerate rich.epub. Run from anywhere: python3 tests/fixtures/epub/make_rich_epub.py"""
import base64, pathlib, zipfile

OUT = pathlib.Path(__file__).parent / "rich.epub"
# 1x1 red PNG
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842"
    "iQAAAABJRU5ErkJggg=="
)

CONTAINER = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf"
    media-type="application/oebps-package+xml"/></rootfiles>
</container>"""

OPF = """<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="uid">rich-test</dc:identifier>
    <dc:title>Rich Book</dc:title>
    <dc:creator>Fixture Author</dc:creator>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
    <item id="img1" href="images/dot.png" media-type="image/png"/>
  </manifest>
  <spine><itemref idref="c1"/><itemref idref="c2"/></spine>
</package>"""

CH1 = """<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>ch1</title></head>
<body>
  <h1>Chapter One</h1>
  <p>Intro with <em>emphasis</em>, <code>inline_code()</code>, and a
     footnote<a epub:type="noteref" href="#fn1">1</a>.</p>
  <ul><li>alpha</li><li>beta<ul><li>beta-one</li></ul></li></ul>
  <table>
    <thead><tr><th>Name</th><th>Value</th></tr></thead>
    <tbody><tr><td>pi</td><td>3.14</td></tr><tr><td>e</td><td>2.72</td></tr></tbody>
  </table>
  <figure>
    <img src="images/dot.png" alt="a dot"/>
    <figcaption>The red dot</figcaption>
  </figure>
  <pre><code class="language-rust">fn main() {}</code></pre>
  <p>See <a href="ch2.xhtml#sect">the second section</a> for more.</p>
  <aside epub:type="footnote" id="fn1"><p>Footnote body text.</p></aside>
</body></html>"""

CH2 = """<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>ch2</title></head>
<body>
  <h1>Chapter Two</h1>
  <p>Opening paragraph of chapter two.</p>
  <h2 id="sect">Second Section</h2>
  <p>Target of the cross-chapter link.</p>
</body></html>"""

with zipfile.ZipFile(OUT, "w", zipfile.ZIP_STORED) as z:
    z.writestr("mimetype", "application/epub+zip")
    z.writestr("META-INF/container.xml", CONTAINER)
    z.writestr("OEBPS/content.opf", OPF)
    z.writestr("OEBPS/ch1.xhtml", CH1)
    z.writestr("OEBPS/ch2.xhtml", CH2)
    z.writestr("OEBPS/images/dot.png", PNG)
print(f"wrote {OUT}")
