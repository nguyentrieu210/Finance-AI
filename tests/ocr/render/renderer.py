"""Bounded Poppler compatibility adapter, synthetic CI only.

Run inside a network-disabled, read-only, non-root container. Never pass real
documents or use this module as a public upload/parse endpoint.
"""
from dataclasses import dataclass, field
import hashlib
import os
from pathlib import Path
import re
import resource
import struct
import subprocess
import tempfile

MAX_BYTES = 25 * 1024 * 1024
MAX_PAGES = 200
MAX_SIDE = 2400
MAX_PNG_BYTES = 24 * 1024 * 1024
MAX_TEXT_BYTES = 262_144
MAX_INFO_BYTES = 65_536


class RenderError(Exception):
    """Fixed diagnostic only; never include document content or tool stderr."""


@dataclass(frozen=True)
class Raster:
    source_sha256: str
    page: int
    page_count: int
    rotation: int
    width: int
    height: int
    png: bytes = field(repr=False)
    text_candidate: str = field(repr=False)


def _limits():
    resource.setrlimit(resource.RLIMIT_CPU, (12, 12))
    resource.setrlimit(resource.RLIMIT_AS, (384 * 1024 * 1024,) * 2)
    resource.setrlimit(resource.RLIMIT_FSIZE, (MAX_PNG_BYTES,) * 2)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def _run(args, directory):
    # Do not capture unbounded subprocess pipes or include stderr in exceptions.
    with tempfile.TemporaryFile(dir=directory) as out, tempfile.TemporaryFile(dir=directory) as err:
        try:
            process = subprocess.run(
                args, cwd=directory, stdin=subprocess.DEVNULL, stdout=out,
                stderr=err, check=False, timeout=20, preexec_fn=_limits,
                env={"PATH": "/usr/bin:/bin", "LC_ALL": "C.UTF-8", "HOME": directory},
            )
        except (OSError, subprocess.SubprocessError) as exc:
            raise RenderError("renderer execution failed") from None
        if process.returncode != 0:
            raise RenderError("renderer rejected input")
        out.seek(0)
        info = out.read(MAX_INFO_BYTES + 1)
        if len(info) > MAX_INFO_BYTES:
            raise RenderError("renderer metadata limit exceeded")
        return info.decode("utf-8", errors="strict")


def _read_bounded(path, maximum):
    with path.open("rb") as handle:
        payload = handle.read(maximum + 1)
    if len(payload) > maximum:
        raise RenderError("renderer output limit exceeded")
    return payload


def _page_count(info):
    match = re.search(r"^Pages:\s+(\d+)\s*$", info, re.MULTILINE)
    if match is None:
        raise RenderError("missing page count")
    pages = int(match[1])
    if not 1 <= pages <= MAX_PAGES:
        raise RenderError("page count exceeds limit")
    if not re.search(r"^Encrypted:\s+no\s*$", info, re.MULTILINE):
        raise RenderError("encrypted or unknown PDF state")
    return pages


def _rotation(info, page):
    match = re.search(
        rf"^Page\s+{page}\s+rot:\s+(\d+)\s*$", info, re.MULTILINE
    ) or re.search(r"^Page rot:\s+(\d+)\s*$", info, re.MULTILINE)
    if match is None or int(match[1]) not in {0, 90, 180, 270}:
        raise RenderError("invalid page rotation")
    return int(match[1])


def _png_dimensions(data):
    if len(data) < 33 or data[:8] != b"\x89PNG\r\n\x1a\n" or data[8:16] != b"\x00\x00\x00\rIHDR":
        raise RenderError("invalid raster output")
    width, height = struct.unpack(">II", data[16:24])
    if not 1 <= width <= MAX_SIDE or not 1 <= height <= MAX_SIDE:
        raise RenderError("raster dimensions exceed limit")
    return width, height


def render_synthetic_page(payload, page):
    """Render one original page, preserving byte hash and explicit rotation.

    Poppler returns display-oriented raster. Downstream must inverse-transform
    OCR boxes before storing unrotated source coordinates. Text layer is only
    a candidate, never proof of visible page content.
    """
    if not isinstance(payload, bytes) or not 1 <= len(payload) <= MAX_BYTES:
        raise RenderError("invalid source size")
    if not payload.startswith(b"%PDF-") or type(page) is not int or not 1 <= page <= MAX_PAGES:
        raise RenderError("invalid source or page")
    try:
        with tempfile.TemporaryDirectory(prefix="finance-render-", dir="/tmp") as directory:
            root = Path(directory)
            source = root / "source.pdf"
            source.write_bytes(payload)
            pages = _page_count(_run(["pdfinfo", str(source)], directory))
            if page > pages:
                raise RenderError("page out of range")
            span = ["-f", str(page), "-l", str(page)]
            rotation = _rotation(_run(["pdfinfo", *span, str(source)], directory), page)
            _run(
                ["pdftoppm", *span, "-scale-to", str(MAX_SIDE), "-png", "-singlefile",
                 str(source), str(root / "raster")], directory
            )
            png = _read_bounded(root / "raster.png", MAX_PNG_BYTES)
            width, height = _png_dimensions(png)
            text_file = root / "text.txt"
            _run(["pdftotext", *span, "-enc", "UTF-8", str(source), str(text_file)], directory)
            text = _read_bounded(text_file, MAX_TEXT_BYTES).decode("utf-8", errors="strict")
            return Raster(hashlib.sha256(payload).hexdigest(), page, pages, rotation,
                          width, height, png, text)
    except (OSError, UnicodeError, ValueError):
        raise RenderError("invalid renderer output") from None
