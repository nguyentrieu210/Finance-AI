#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export OMP_THREAD_LIMIT=2
# No real data or provider calls. Font and model are distro test assets.
python3 - "$work" <<'PY'
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
import sys
root = Path(sys.argv[1])
im = Image.new("RGB", (1800, 480), "white")
draw = ImageDraw.Draw(im)
font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 68)
draw.text((90, 90), "Tổng thanh toán", font=font, fill="black")
draw.text((90, 240), "125000 đồng", font=font, fill="black")
im.save(root / "synthetic.png")
PY
tesseract --version
tesseract --list-langs | grep -Fx vie
# Hard wall-clock bound and bounded process output. Fixture only, not a sandbox for arbitrary PDFs.
(
  ulimit -v 2097152
  ulimit -f 4096
  timeout --kill-after=2s 30s tesseract "$work/synthetic.png" "$work/result" -l vie --psm 6 tsv
)
FINANCE_OCR_TSV="$work/result.tsv" cargo test --locked -p finance-ocr --test real_tsv -- --ignored --exact real_vietnamese_tsv
echo 'PASS: Vietnamese synthetic image -> actual Tesseract -> Rust bounded TSV candidate'
