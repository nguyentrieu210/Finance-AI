"""Synthetic image diagnostics, not an approved acceptance or release policy.

Bounded grayscale raster in memory only. No external I/O, model or OCR
invocation. Coordinates from corrected images must be mapped to original
pixels using the returned inverse affine transform before becoming evidence.
"""
from dataclasses import dataclass, field
import math

import cv2
import numpy as np

MAX_SIDE = 2400
MAX_PIXELS = MAX_SIDE * MAX_SIDE
MAX_ANGLE = 10.0
# Provisional synthetic-only heuristics; no calibrated financial thresholds.
MIN_CONTRAST = 45.0
MIN_SHARPNESS = 35.0
MIN_INK = 0.002
MAX_INK = 0.45
MIN_DESKEW = 0.75


@dataclass(frozen=True)
class Diagnostics:
    contrast: float
    sharpness: float
    ink_fraction: float
    correction_degrees: float | None
    reasons: tuple[str, ...]
    requires_review: bool = True


@dataclass(frozen=True)
class Corrected:
    raster: np.ndarray = field(repr=False)
    source_to_output: np.ndarray = field(repr=False)
    output_to_source: np.ndarray = field(repr=False)
    correction_degrees: float


def validate(raster):
    if not isinstance(raster, np.ndarray) or raster.dtype != np.uint8 or raster.ndim != 2:
        raise ValueError("expected grayscale uint8 raster")
    height, width = raster.shape
    if not 32 <= width <= MAX_SIDE or not 32 <= height <= MAX_SIDE or raster.size > MAX_PIXELS:
        raise ValueError("raster bounds")
    return np.ascontiguousarray(raster)


def _foreground(raster):
    _, ink = cv2.threshold(raster, 0, 255, cv2.THRESH_BINARY_INV | cv2.THRESH_OTSU)
    return ink


def _correction(ink):
    # Projection-profile search on a bounded, downsampled raster. Score
    # normalized row-energy; abstain when there is insufficient signal or
    # the best angle is on the boundary. Not suitable for arbitrary layouts.
    height, width = ink.shape
    scale = min(1.0, 900.0 / max(height, width))
    reduced = cv2.resize(ink, None, fx=scale, fy=scale, interpolation=cv2.INTER_AREA)
    h, w = reduced.shape
    center = ((w - 1) / 2.0, (h - 1) / 2.0)
    scores = []
    for tick in range(-20, 21):
        angle = tick / 2.0
        matrix = cv2.getRotationMatrix2D(center, angle, 1.0)
        transformed = cv2.warpAffine(
            reduced, matrix, (w, h), flags=cv2.INTER_NEAREST,
            borderMode=cv2.BORDER_CONSTANT, borderValue=0,
        )
        rows = transformed.astype(np.float64).sum(axis=1)
        total = rows.sum()
        score = float(np.dot(rows, rows) / (total * total)) if total > 0 else 0.0
        scores.append((score, angle))
    peak, angle = max(scores, key=lambda item: (item[0], -abs(item[1])))
    baseline = scores[20][0]
    if peak <= 0 or abs(angle) >= MAX_ANGLE:
        return None
    if peak < baseline * 1.02 or abs(angle) < MIN_DESKEW:
        return 0.0
    return angle


def diagnose(raster):
    raster = validate(raster)
    cv2.setNumThreads(1)
    ink = _foreground(raster)
    ink_fraction = float(np.count_nonzero(ink) / raster.size)
    low, high = np.percentile(raster, [1, 99])
    contrast = float(high - low)
    sharpness = float(cv2.Laplacian(raster, cv2.CV_64F).var())
    reasons = []
    if ink_fraction < MIN_INK or ink_fraction > MAX_INK:
        reasons.append("INSUFFICIENT_OR_EXCESS_INK")
    if contrast < MIN_CONTRAST:
        reasons.append("LOW_CONTRAST")
    if sharpness < MIN_SHARPNESS:
        reasons.append("LOW_SHARPNESS")
    correction = None
    if not reasons:
        correction = _correction(ink)
        if correction is None:
            reasons.append("SKEW_UNCERTAIN")
        elif abs(correction) >= MIN_DESKEW:
            reasons.append("DESKEW_CANDIDATE")
    return Diagnostics(contrast, sharpness, ink_fraction, correction, tuple(reasons))


def correct(raster, correction_degrees):
    raster = validate(raster)
    if type(correction_degrees) not in (int, float) or not math.isfinite(correction_degrees):
        raise ValueError("invalid angle")
    if abs(correction_degrees) >= MAX_ANGLE:
        raise ValueError("angle out of bounds")
    height, width = raster.shape
    matrix = cv2.getRotationMatrix2D(((width - 1) / 2.0, (height - 1) / 2.0),
                                     correction_degrees, 1.0)
    # Expand output canvas to preserve all source pixel centers, including
    # margins. Refuse oversized derived images rather than crop evidence.
    corners = np.array([[0, 0, 1], [width - 1, 0, 1],
                        [0, height - 1, 1], [width - 1, height - 1, 1]])
    mapped = corners @ matrix.T
    lower = np.floor(mapped.min(axis=0))
    upper = np.ceil(mapped.max(axis=0))
    out_width, out_height = (int(n) + 1 for n in upper - lower)
    if max(out_width, out_height) > MAX_SIDE or out_width * out_height > MAX_PIXELS:
        raise ValueError("expanded raster bounds")
    matrix[:, 2] -= lower
    output = cv2.warpAffine(
        raster, matrix, (out_width, out_height), flags=cv2.INTER_LINEAR,
        borderMode=cv2.BORDER_CONSTANT, borderValue=255,
    )
    inverse = cv2.invertAffineTransform(matrix)
    return Corrected(output, matrix, inverse, float(correction_degrees))
