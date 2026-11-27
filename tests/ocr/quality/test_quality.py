import unittest

import cv2
import numpy as np
from PIL import Image, ImageDraw, ImageFont

from quality import correct, diagnose, validate


def fixture():
    image = Image.new("L", (1200, 800), 255)
    font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 38)
    draw = ImageDraw.Draw(image)
    for index in range(7):
        draw.text((120, 120 + index * 75),
                  f"Tổng thanh toán {125000 + index * 1000}",
                  font=font, fill=0)
    return np.array(image)


class QualityTests(unittest.TestCase):
    def test_sharp_and_blurred_candidates(self):
        source = fixture()
        sharp = diagnose(source)
        blurred = diagnose(cv2.GaussianBlur(source, (31, 31), 7))
        self.assertGreater(sharp.sharpness, blurred.sharpness)
        self.assertIn("LOW_SHARPNESS", blurred.reasons)
        self.assertTrue(sharp.requires_review)
        self.assertTrue(blurred.requires_review)

    def test_blank_and_low_contrast(self):
        blank = diagnose(np.full((100, 100), 255, dtype=np.uint8))
        self.assertIn("INSUFFICIENT_OR_EXCESS_INK", blank.reasons)
        self.assertIsNone(blank.correction_degrees)
        low = (fixture().astype(np.float64) * (20 / 255) + 190).astype(np.uint8)
        result = diagnose(low)
        self.assertIn("LOW_CONTRAST", result.reasons)
        self.assertTrue(result.requires_review)

    def test_synthetic_skew_and_inverse_mapping(self):
        source = fixture()
        h, w = source.shape
        skew = cv2.getRotationMatrix2D(((w - 1) / 2, (h - 1) / 2), 5.0, 1.0)
        rotated = cv2.warpAffine(source, skew, (w, h), borderValue=255)
        result = diagnose(rotated)
        self.assertIsNotNone(result.correction_degrees)
        self.assertAlmostEqual(result.correction_degrees, -5.0, delta=1.0)
        self.assertIn("DESKEW_CANDIDATE", result.reasons)
        corrected = correct(rotated, result.correction_degrees)
        self.assertGreater(corrected.raster.size, rotated.size)
        for point in ([0, 0], [w-1, h-1], [w/2, h/2]):
            mapped = corrected.source_to_output @ np.array([*point, 1])
            recovered = corrected.output_to_source @ np.array([*mapped, 1])
            np.testing.assert_allclose(recovered, point, atol=1e-6)
            self.assertGreaterEqual(mapped.min(), -1e-6)
            self.assertLessEqual(mapped[0], corrected.raster.shape[1] - 1 + 1e-6)
            self.assertLessEqual(mapped[1], corrected.raster.shape[0] - 1 + 1e-6)
        np.testing.assert_allclose(diagnose(corrected.raster).correction_degrees, 0, atol=1)

    def test_reject_invalid_inputs(self):
        for bad in (np.zeros((10, 10), dtype=np.uint8),
                    np.zeros((32, 2401), dtype=np.uint8),
                    np.zeros((32, 32, 3), dtype=np.uint8),
                    np.zeros((32, 32), dtype=np.float32)):
            with self.assertRaises(ValueError):
                validate(bad)
        for bad in (float("nan"), 10.0, -10.0, True):
            with self.assertRaises(ValueError):
                correct(fixture(), bad)


if __name__ == "__main__":
    unittest.main()
