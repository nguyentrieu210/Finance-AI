from pathlib import Path
import yaml

root = Path(__file__).resolve().parents[1]
manifest = yaml.safe_load((root / "tests/system/gates.yaml").read_text())
workflow = yaml.safe_load((root / ".github/workflows/foundation.yml").read_text())
jobs = set(workflow["jobs"])
assert manifest["version"] == 1
required = {"G02", "G03", "G04_TECHNICAL"}
assert set(manifest["gates"]) == required
for name, gate in manifest["gates"].items():
    assert gate["kind"] == "technical"
    evidence = gate["evidence"]
    assert evidence and len(evidence) == len(set(evidence))
    missing = set(evidence) - jobs
    assert not missing, f"{name}: unknown workflow jobs {sorted(missing)}"
    assert gate.get("proves"), f"{name}: no proof declaration"
blocked = {item["id"] for item in manifest["external_dependencies"] if item["state"] == "blocked"}
assert {"CORPUS", "FINANCE_SIGNOFF", "SECURITY_SIGNOFF", "X01"} <= blocked
assert "G04_TECHNICAL" in manifest["gates"]
assert "G04" not in manifest["gates"], "full G04 must not be claimed without external evidence"
print("PASS: system gate manifest references real CI jobs and keeps external release dependencies fail-closed")
