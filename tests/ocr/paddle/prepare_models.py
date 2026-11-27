"""Download official public compatibility weights at image build, before any input.
No approved digest exists yet: record actual weights; do not claim production pinning.
"""
import hashlib
from pathlib import Path
import shutil
import tarfile
import urllib.request

ROOT = Path("/models")
BASE = "https://paddle-model-ecology.bj.bcebos.com/paddlex/official_inference_model/paddle3.0.0/"
MODELS = ("PP-OCRv5_mobile_det", "latin_PP-OCRv5_mobile_rec")
CAP = 100 * 1024 * 1024


def download(name):
    archive = ROOT / (name + ".tar")
    with urllib.request.urlopen(BASE + name + "_infer.tar", timeout=60) as response:
        with archive.open("wb") as target:
            count = 0
            while chunk := response.read(1024 * 1024):
                count += len(chunk)
                if count > CAP:
                    raise ValueError("model archive too large")
                target.write(chunk)
    dest = ROOT / name
    dest.mkdir()
    with tarfile.open(archive, "r:*") as package:
        total = 0
        for member in package.getmembers():
            if member.isdir():
                continue
            if not member.isfile():
                raise ValueError("unexpected model archive member")
            total += member.size
            if total > CAP:
                raise ValueError("expanded model archive too large")
            # Only regular model files, flattened to a private directory.
            filename = Path(member.name).name
            if filename not in {"inference.json", "inference.pdiparams", "inference.yml", "inference.pdmodel"}:
                continue
            with package.extractfile(member) as source, (dest / filename).open("xb") as target:
                shutil.copyfileobj(source, target)
    archive.unlink()
    required = {"inference.yml", "inference.pdiparams"}
    names = {p.name for p in dest.iterdir()}
    if not required <= names or not {"inference.json", "inference.pdmodel"} & names:
        raise ValueError("missing model artifacts")
    for path in sorted(dest.iterdir()):
        print(name, path.name, hashlib.sha256(path.read_bytes()).hexdigest())


ROOT.mkdir()
for model in MODELS:
    download(model)
