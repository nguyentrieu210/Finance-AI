"""Actual offline synthetic PaddleOCR inference; emit only bounded JSON."""
import contextlib
import hashlib
import importlib.metadata
import json
import math
from pathlib import Path
import sys

WIDTH, HEIGHT = 1800, 480
ROOT = Path("/models")
DETECT = "PP-OCRv5_mobile_det"
RECOGNIZE = "latin_PP-OCRv5_mobile_rec"
OPTIONS = {
    "text_detection_model_name": DETECT,
    "text_detection_model_dir": str(ROOT / DETECT),
    "text_recognition_model_name": RECOGNIZE,
    "text_recognition_model_dir": str(ROOT / RECOGNIZE),
    "use_doc_orientation_classify": False,
    "use_doc_unwarping": False,
    "use_textline_orientation": False,
    "device": "cpu",
    "enable_mkldnn": False,
    "cpu_threads": 2,
}


def model_digest():
    digest = hashlib.sha256()
    for path in sorted(ROOT.glob("*/*")):
        name = str(path.relative_to(ROOT)).encode("utf-8")
        digest.update(len(name).to_bytes(4, "big"))
        digest.update(name)
        digest.update(hashlib.sha256(path.read_bytes()).digest())
    return digest.hexdigest()


def run():
    # SDK output belongs to stderr, never the machine-readable result channel.
    with contextlib.redirect_stdout(sys.stderr):
        from PIL import Image, ImageDraw, ImageFont
        from paddleocr import PaddleOCR
        image = Image.new("RGB", (WIDTH, HEIGHT), "white")
        draw = ImageDraw.Draw(image)
        font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 68)
        draw.text((90, 90), "Tổng thanh toán", font=font, fill="black")
        draw.text((90, 240), "125000 đồng", font=font, fill="black")
        path = Path("/tmp/synthetic.png")
        image.save(path)
        ocr = PaddleOCR(**OPTIONS)
        results = list(ocr.predict(str(path)))
    if len(results) != 1:
        raise ValueError("expected one synthetic page")
    result = results[0]
    texts, scores, polys = result["rec_texts"], result["rec_scores"], result["rec_polys"]
    if not 0 < len(texts) == len(scores) == len(polys) <= 10000:
        raise ValueError("inconsistent detection arrays")
    detections = []
    total = 0
    for text, score, poly in zip(texts, scores, polys):
        if not isinstance(text, str) or not text.strip() or len(text.encode("utf-8")) > 4096:
            raise ValueError("invalid candidate text")
        total += len(text.encode("utf-8"))
        if total > 262144:
            raise ValueError("text limit")
        score = float(score)
        if not math.isfinite(score) or not 0 <= score <= 1:
            raise ValueError("invalid confidence")
        if len(poly) != 4 or any(len(point) != 2 for point in poly):
            raise ValueError("invalid polygon")
        points = []
        for point in poly:
            x, y = (float(n) for n in point)
            if not all(math.isfinite(n) and n.is_integer() for n in (x, y)):
                raise ValueError("noninteger polygon")
            if not 0 <= x <= WIDTH or not 0 <= y <= HEIGHT:
                raise ValueError("out of frame")
            points.append([int(x), int(y)])
        detections.append({"points": points, "text": text, "confidence_bps": int(score * 10000)})
    version = importlib.metadata.version("paddleocr")
    runtime = importlib.metadata.version("paddlepaddle")
    config = json.dumps({"options": OPTIONS, "paddleocr": version, "runtime": runtime}, sort_keys=True)
    payload = {
        "schema": "finance.ocr.synthetic.v1",
        "version": version,
        "runtime": runtime,
        "model_sha256": model_digest(),
        "config_sha256": hashlib.sha256(config.encode()).hexdigest(),
        "raster_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "width": WIDTH, "height": HEIGHT, "detections": detections,
    }
    output = json.dumps(payload, ensure_ascii=False, allow_nan=False).encode("utf-8")
    if len(output) > 2 * 1024 * 1024:
        raise ValueError("output limit")
    sys.stdout.buffer.write(output)


if __name__ == "__main__":
    run()
