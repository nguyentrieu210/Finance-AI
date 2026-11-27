import hashlib
import unittest

from renderer import MAX_PAGES, MAX_SIDE, RenderError, render_synthetic_page


def fixture(page_count=2, rotated=True):
    # Small deterministic PDF with explicit xref; no external files or fonts.
    objects = {}
    objects[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
    children = " ".join(f"{4+2*i} 0 R" for i in range(page_count))
    objects[2] = f"<< /Type /Pages /Count {page_count} /Kids [{children}] >>".encode()
    objects[3] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
    for i in range(page_count):
        page_id = 4 + 2*i
        rotation = 90 if rotated and i == 1 else 0
        objects[page_id] = (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Rotate {rotation} "
            f"/Resources << /Font << /F1 3 0 R >> >> /Contents {page_id+1} 0 R >>"
        ).encode()
        amount = 125000 if i == 0 else 230000
        content = f"BT /F1 18 Tf 20 100 Td (TOTAL {amount}) Tj ET".encode()
        objects[page_id+1] = (
            f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"\nendstream"
        )
    data = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for number in sorted(objects):
        offsets.append(len(data))
        data.extend(f"{number} 0 obj\n".encode() + objects[number] + b"\nendobj\n")
    start = len(data)
    data.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        data.extend(f"{offset:010d} 00000 n \n".encode())
    data.extend(
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n".encode()
    )
    return bytes(data)


class RenderTests(unittest.TestCase):
    def test_exact_original_hash_and_separate_pages(self):
        payload = fixture()
        first = render_synthetic_page(payload, 1)
        second = render_synthetic_page(payload, 2)
        self.assertEqual(first.source_sha256, hashlib.sha256(payload).hexdigest())
        self.assertEqual(second.source_sha256, first.source_sha256)
        self.assertEqual((first.page, second.page, first.page_count), (1, 2, 2))
        self.assertIn("125000", first.text_candidate)
        self.assertNotIn("230000", first.text_candidate)
        self.assertIn("230000", second.text_candidate)
        self.assertNotIn("125000", second.text_candidate)
        self.assertEqual(first.rotation, 0)
        self.assertEqual(second.rotation, 90)
        self.assertGreater(first.width, first.height)
        self.assertGreater(second.height, second.width)
        self.assertLessEqual(max(first.width, first.height), MAX_SIDE)
        self.assertLessEqual(max(second.width, second.height), MAX_SIDE)
        self.assertNotEqual(first.png, second.png)

    def test_malformed_and_invalid_page_fail_closed(self):
        for data, page in [(b"%PDF-1.4\ninvalid", 1), (b"not pdf", 1),
                           (fixture(), 0), (fixture(), 3), (fixture(), True)]:
            with self.subTest(page=page, bytes=len(data)):
                with self.assertRaises(RenderError):
                    render_synthetic_page(data, page)

    def test_page_limit_before_render(self):
        with self.assertRaisesRegex(RenderError, "page count exceeds limit"):
            render_synthetic_page(fixture(MAX_PAGES+1, False), 1)


if __name__ == "__main__":
    unittest.main()
