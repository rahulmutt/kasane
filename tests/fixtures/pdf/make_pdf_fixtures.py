#!/usr/bin/env python3
"""Hermetic PDF fixture generator (stdlib only). Regenerate with:
    python3 tests/fixtures/pdf/make_pdf_fixtures.py
Emits minimal.pdf, no-outline.pdf, image.pdf, scanned.pdf next to this file.
"""
import os
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))


class Pdf:
    """Minimal PDF writer that tracks object byte offsets for the xref table."""

    def __init__(self):
        self.objects = {}   # num -> bytes (object body, without "N 0 obj"/"endobj")
        self.order = []     # emission order of object numbers

    def add(self, num, body: bytes):
        self.objects[num] = body
        self.order.append(num)

    def stream_obj(self, dict_extra: bytes, data: bytes) -> bytes:
        return b"<< /Length %d %s >>\nstream\n%s\nendstream" % (len(data), dict_extra, data)

    def build(self, root_num: int) -> bytes:
        out = bytearray(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n")
        offsets = {}
        for num in self.order:
            offsets[num] = len(out)
            out += b"%d 0 obj\n" % num
            out += self.objects[num]
            out += b"\nendobj\n"
        xref_pos = len(out)
        max_num = max(self.order)
        out += b"xref\n0 %d\n" % (max_num + 1)
        out += b"0000000000 65535 f \n"
        for num in range(1, max_num + 1):
            if num in offsets:
                out += b"%010d 00000 n \n" % offsets[num]
            else:
                out += b"0000000000 65535 f \n"
        out += b"trailer\n<< /Size %d /Root %d 0 R >>\n" % (max_num + 1, root_num)
        out += b"startxref\n%d\n%%%%EOF" % xref_pos
        return bytes(out)


def text_stream(ops: bytes) -> bytes:
    return b"BT\n" + ops + b"ET\n"


def show(x, y, size, s: str) -> bytes:
    esc = s.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
    return b"/F1 %d Tf\n1 0 0 1 %d %d Tm\n(%s) Tj\n" % (size, x, y, esc.encode("latin-1"))


def rgb_image_stream(w: int, h: int) -> bytes:
    # w*h RGB pixels, deflate-compressed => /Filter /FlateDecode.
    raw = bytes([((i * 37) % 256) for i in range(w * h * 3)])
    return zlib.compress(raw)


def font_obj() -> bytes:
    return b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"


def build_minimal() -> bytes:
    p = Pdf()
    # 1 catalog, 2 pages tree, 3+4 page, 5+6 content, 7 font, 8 outlines, 9+10 outline items
    p.add(1, b"<< /Type /Catalog /Pages 2 0 R /Outlines 8 0 R >>")
    p.add(2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>")
    p.add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
             b"/Resources << /Font << /F1 7 0 R >> >> /Contents 5 0 R >>")
    p.add(4, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
             b"/Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>")
    c1 = text_stream(show(20, 170, 12, "Chapter One") + show(20, 150, 12, "First body line."))
    c2 = text_stream(show(20, 170, 12, "Section Two") + show(20, 150, 12, "Second body line."))
    p.add(5, p.stream_obj(b"", c1))
    p.add(6, p.stream_obj(b"", c2))
    p.add(7, font_obj())
    p.add(8, b"<< /Type /Outlines /First 9 0 R /Last 10 0 R /Count 2 >>")
    p.add(9, b"<< /Title (Chapter One) /Parent 8 0 R /Next 10 0 R /Dest [3 0 R /Fit] >>")
    p.add(10, b"<< /Title (Section Two) /Parent 8 0 R /Prev 9 0 R /Dest [4 0 R /Fit] >>")
    return p.build(1)


def build_no_outline() -> bytes:
    p = Pdf()
    p.add(1, b"<< /Type /Catalog /Pages 2 0 R >>")
    p.add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    p.add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
             b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>")
    body = (show(20, 170, 24, "Big Title")
            + show(20, 140, 12, "Ordinary paragraph text one.")
            + show(20, 124, 12, "Ordinary paragraph text two."))
    p.add(4, font_obj())
    p.add(5, p.stream_obj(b"", text_stream(body)))
    return p.build(1)


def build_image(with_text: bool) -> bytes:
    p = Pdf()
    p.add(1, b"<< /Type /Catalog /Pages 2 0 R >>")
    p.add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    p.add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
             b"/Resources << /Font << /F1 4 0 R >> /XObject << /Im0 6 0 R >> >> "
             b"/Contents 5 0 R >>")
    p.add(4, font_obj())
    ops = b"q 100 0 0 100 20 20 cm /Im0 Do Q\n"
    if with_text:
        ops = text_stream(show(20, 170, 12, "Figure caption text.")) + ops
    p.add(5, p.stream_obj(b"", ops))
    img = rgb_image_stream(2, 2)
    p.add(6, p.stream_obj(
        b"/Type /XObject /Subtype /Image /Width 2 /Height 2 "
        b"/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode", img))
    return p.build(1)


def main():
    open(os.path.join(HERE, "minimal.pdf"), "wb").write(build_minimal())
    open(os.path.join(HERE, "no-outline.pdf"), "wb").write(build_no_outline())
    open(os.path.join(HERE, "image.pdf"), "wb").write(build_image(with_text=True))
    open(os.path.join(HERE, "scanned.pdf"), "wb").write(build_image(with_text=False))
    print("wrote minimal.pdf, no-outline.pdf, image.pdf, scanned.pdf")


if __name__ == "__main__":
    main()
