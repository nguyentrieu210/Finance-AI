"""Check technical draft conventions without asserting business approval."""
from pathlib import Path
import yaml
from openapi_parameters import HTTP_METHODS, MUTATING_METHODS, effective_parameters, validate_operation_parameters

root = Path(__file__).resolve().parents[1]
api = yaml.safe_load((root / "contracts/openapi.yaml").read_text())
targets = {"dossiers", "organizations", "memberships", "policies", "rules",
           "forms", "templates", "workflows", "references", "risks", "reconciliation"}
commands = versioned = 0
for path, item in api["paths"].items():
    for method, operation in item.items():
        if method not in HTTP_METHODS:
            continue
        validate_operation_parameters(path, method, item, operation)
        params = {p["name"]: p for p in effective_parameters(item, operation)}
        responses = [v for k, v in operation["responses"].items() if str(k).startswith("2")]
        assert "If-Match" not in params, path
        assert all("ETag" not in v.get("headers", {}) for v in responses), path
        if method in MUTATING_METHODS:
            commands += 1
            if operation.get("x-idempotent") is not True:
                key = params["Idempotency-Key"]
                assert key["required"] and key["schema"]["maxLength"] == 128
                assert key["schema"]["pattern"] == r"^[A-Za-z0-9._~-]+$"
        if "If-Finance-Version" in params:
            versioned += 1
            assert params["If-Finance-Version"]["required"]
            assert path.split("/")[2] in targets
            assert all(v["headers"]["Finance-Version"]["required"] for v in responses)
            assert {"400", "409"} <= operation["responses"].keys()
        if method == "get" and path in ("/v1/dossiers/{dossier_id}", "/v1/policies/{item_id}"):
            assert all(v["headers"]["Finance-Version"]["required"] for v in responses)
        if method == "get" and path in ("/v1/dossiers/{dossier_id}/documents", "/v1/evidence/{evidence_id}"):
            assert all("Finance-Version" not in v.get("headers", {}) for v in responses)
        if method == "get" and path == "/v1/dossiers":
            assert {"company_id", "kind", "status", "updated_from", "updated_to"} <= params.keys()
assert commands and versioned
print(f"PASS: {commands} idempotent command contracts, {versioned} aggregate version guards; no HTTP ETag conflation.")

create = api["paths"]["/v1/dossiers/{dossier_id}/documents"]["post"]
grant = api["paths"]["/v1/dossiers/{dossier_id}/documents/{document_id}/upload-grants"]["post"]
final = api["paths"]["/v1/dossiers/{dossier_id}/documents/{document_id}/finalize"]["post"]
schemas = api["components"]["schemas"]
assert "upload_url" not in schemas["UploadIntent"]["properties"]
assert set(schemas["UploadGrant"]["required"]) == {"document_id", "generation", "upload_url", "expires_at", "content_type", "checksum_sha256_base64"}
assert grant["responses"]["201"]["content"]["application/json"]["schema"]["$ref"].endswith("/UploadGrant")
assert "UPLOAD_GRANT_INACTIVE" in grant["responses"]["409"]["description"]
assert set(final["requestBody"]["content"]["application/json"]["schema"]["required"]) == {"generation", "sha256"}
assert "PENDING_UPLOAD" in schemas["Document"]["properties"]["state"]["enum"]
print("PASS: metadata-only create intent, separate fenced upload grant and generation-bound finalize contract.")
