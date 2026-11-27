"""Real MinIO capability probe on disposable GitHub-hosted runner only.

Independent Python test harness; this is not the Rust production provider adapter.
No real financial files, network egress to an external object store, or credentials.
"""
import base64
from concurrent.futures import ThreadPoolExecutor
import hashlib
import http.client
import os
import sys
import time
from urllib.parse import parse_qs, urlsplit
from uuid import UUID, uuid4

import boto3
from botocore.config import Config
from botocore.exceptions import ClientError

MAX_BYTES = 25 * 1024 * 1024
MIME = "application/pdf"
STAGING = "finance-synthetic-staging"
QUARANTINE = "finance-synthetic-quarantine"
DATA = b"%PDF-1.7\nSynthetic provider probe, not a parsed PDF.\n"
OTHER = DATA[:-2] + b"!\n"
STAGE = "configuration"


def require(condition, label):
    if not condition:
        raise AssertionError(label)


def sha256_b64(data):
    return base64.b64encode(hashlib.sha256(data).digest()).decode("ascii")


def scoped_key(phase, tenant, company, revision, document, generation):
    require(phase in {"staging", "quarantine"}, "invalid phase")
    for value in (tenant, company, revision, document):
        require(isinstance(value, UUID) and value.int != 0, "invalid key scope")
    require(1 <= generation <= 9007199254740991, "invalid generation")
    return f"{phase}/{tenant}/{company}/{revision}/{document}/{generation}"


def loopback_parts(url, endpoint):
    result = urlsplit(url)
    configured = urlsplit(endpoint)
    require(result.scheme == "http" and result.hostname == "127.0.0.1", "non-loopback URL")
    require(result.netloc == configured.netloc and result.port == configured.port, "unexpected provider address")
    require(not result.username and not result.password and not result.fragment, "invalid provider URL")
    return result


def raw_request(endpoint, url, method, body=None, headers=None):
    target = loopback_parts(url, endpoint)
    path = target.path + ("?" + target.query if target.query else "")
    conn = http.client.HTTPConnection(target.hostname, target.port, timeout=10)
    try:
        conn.request(method, path, body=body, headers=headers or {})
        reply = conn.getresponse()
        status = reply.status
        response_headers = dict((k.lower(), v) for k, v in reply.getheaders())
        reply.read(8192)  # never print response body or presigned URL
        return status, response_headers
    finally:
        conn.close()


def signed_put(client, key, data, ttl=60):
    require(1 <= len(data) <= MAX_BYTES and 1 <= ttl <= 300, "invalid upload bounds")
    params = dict(Bucket=STAGING, Key=key, ContentLength=len(data),
                  ContentType=MIME, ChecksumSHA256=sha256_b64(data), IfNoneMatch="*")
    url = client.generate_presigned_url("put_object", Params=params, ExpiresIn=ttl, HttpMethod="PUT")
    signed = set(parse_qs(urlsplit(url).query).get("X-Amz-SignedHeaders", [""])[0].split(";"))
    require({"content-length", "content-type", "host", "x-amz-checksum-sha256", "if-none-match"} <= signed,
            "signed grant missing required constraints")
    headers = {"Content-Length": str(len(data)), "Content-Type": MIME,
               "x-amz-checksum-sha256": sha256_b64(data), "If-None-Match": "*"}
    return url, headers


def version_id(value):
    require(isinstance(value, str) and 0 < len(value) <= 1024 and value != "null", "missing immutable version ID")
    return value


def read_verified(client, bucket, key, version, expected_length, expected_sha):
    require(0 < expected_length <= MAX_BYTES, "invalid read bound")
    result = client.get_object(Bucket=bucket, Key=key, VersionId=version_id(version))
    body = result["Body"]
    try:
        require(result.get("VersionId") == version, "provider returned wrong version")
        require(result["ContentLength"] == expected_length, "provider length mismatch")
        require(result["ContentType"] == MIME, "provider MIME metadata mismatch")
        data = body.read(expected_length + 1)
        require(len(data) == expected_length, "actual length mismatch")
        require(body.read(1) == b"", "unexpected trailing bytes")
        require(hashlib.sha256(data).digest() == expected_sha, "actual digest mismatch")
        require(data.startswith(b"%PDF-"), "signature mismatch")
        return data
    finally:
        body.close()


def promote(client, key, data):
    # Put verified bytes, not a copy from a mutable staging alias.
    return client.put_object(Bucket=QUARANTINE, Key=key, Body=data,
                             ContentLength=len(data), ContentType=MIME,
                             ChecksumSHA256=sha256_b64(data), IfNoneMatch="*")


def conditional_attempt(client, key, data):
    try:
        return ("success", version_id(promote(client, key, data).get("VersionId")))
    except ClientError as exc:
        status = exc.response["ResponseMetadata"]["HTTPStatusCode"]
        require(status in {409, 412}, "unexpected conditional write failure")
        return ("rejected", None)


def run():
    global STAGE
    endpoint = os.environ["FINANCE_S3_PROBE_ENDPOINT"]
    parts = loopback_parts(endpoint, endpoint)
    require(parts.path in {"", "/"} and not parts.query, "invalid endpoint")
    require(len(DATA) == len(OTHER) and DATA != OTHER, "invalid synthetic fixture")
    client = boto3.client("s3", endpoint_url=endpoint, region_name="us-east-1",
                          aws_access_key_id=os.environ["MINIO_ROOT_USER"],
                          aws_secret_access_key=os.environ["MINIO_ROOT_PASSWORD"],
                          config=Config(signature_version="s3v4", s3={"addressing_style": "path"},
                                        connect_timeout=5, read_timeout=10,
                                        retries={"total_max_attempts": 1},
                                        request_checksum_calculation="when_required",
                                        response_checksum_validation="when_required"))
    STAGE = "versioning"
    for bucket in (STAGING, QUARANTINE):
        client.create_bucket(Bucket=bucket)
        client.put_bucket_versioning(Bucket=bucket, VersioningConfiguration={"Status": "Enabled"})
        require(client.get_bucket_versioning(Bucket=bucket).get("Status") == "Enabled", "versioning disabled")
    tenant, company, revision, document = (uuid4() for _ in range(4))
    key = scoped_key("staging", tenant, company, revision, document, 1)
    next_key = scoped_key("staging", tenant, company, revision, document, 2)
    original_key = scoped_key("quarantine", tenant, company, revision, document, 1)
    require(key != next_key and key != original_key, "generation/phase alias")
    expected = hashlib.sha256(DATA).digest()

    STAGE = "signed upload constraints"
    url, headers = signed_put(client, key, DATA)
    for bad_data, bad_headers in (
        (OTHER, headers),
        (DATA + b"x", {**headers, "Content-Length": str(len(DATA) + 1)}),
        (DATA, {**headers, "Content-Type": "text/plain"}),
    ):
        status, _ = raw_request(endpoint, url, "PUT", bad_data, bad_headers)
        require(400 <= status < 500, "provider accepted invalid signed upload")
    require(not client.list_object_versions(Bucket=STAGING, Prefix=key).get("Versions"),
            "invalid upload persisted a version")
    status, reply = raw_request(endpoint, url, "PUT", DATA, headers)
    require(status == 200, "valid signed upload rejected")
    first = version_id(reply.get("x-amz-version-id"))
    status, _ = raw_request(endpoint, url, "PUT", DATA, headers)
    require(status in {409, 412}, "same grant overwrote object")

    STAGE = "private storage and immutable version read"
    status, _ = raw_request(endpoint, f"{endpoint}/{STAGING}/{key}", "GET")
    require(status in {403, 404}, "anonymous staging read allowed")
    verified = read_verified(client, STAGING, key, first, len(DATA), expected)
    # Deliberate privileged fixture write to mutable alias: exact-version read must still select first.
    changed = client.put_object(Bucket=STAGING, Key=key, Body=OTHER,
                                ContentLength=len(OTHER), ContentType=MIME,
                                ChecksumSHA256=sha256_b64(OTHER))
    require(version_id(changed.get("VersionId")) != first, "provider reused version ID")
    require(read_verified(client, STAGING, key, first, len(DATA), expected) == verified,
            "staged version changed after alias overwrite")

    STAGE = "conditional quarantine promotion"
    with ThreadPoolExecutor(max_workers=2) as pool:
        outcomes = list(pool.map(lambda _: conditional_attempt(client, original_key, verified), range(2)))
    successful = [v for result, v in outcomes if result == "success"]
    require(len(successful) == 1 and sum(result == "rejected" for result, _ in outcomes) == 1,
            "concurrent original creation was not exclusive")
    original_version = successful[0]
    require(read_verified(client, QUARANTINE, original_key, original_version, len(DATA), expected) == DATA,
            "original read-back mismatch")
    require(conditional_attempt(client, original_key, OTHER)[0] == "rejected", "original overwrite allowed")
    versions = client.list_object_versions(Bucket=QUARANTINE, Prefix=original_key).get("Versions", [])
    require(len(versions) == 1 and versions[0]["VersionId"] == original_version,
            "unexpected quarantine versions")
    status, _ = raw_request(endpoint, f"{endpoint}/{QUARANTINE}/{original_key}", "GET")
    require(status in {403, 404}, "anonymous quarantine read allowed")

    STAGE = "grant expiration"
    expiring_url, expiring_headers = signed_put(client, next_key, DATA, ttl=1)
    time.sleep(3)
    status, _ = raw_request(endpoint, expiring_url, "PUT", DATA, expiring_headers)
    require(400 <= status < 500, "expired grant accepted")
    require(not client.list_object_versions(Bucket=STAGING, Prefix=next_key).get("Versions"),
            "expired grant persisted a version")
    print("PASS: real S3-compatible provider signed bounds/checksum, private buckets, exact version reads, concurrent conditional quarantine, expiry")
    print("No production authorization, object-lock, scanner, DB attachment, or deletion/retention tested.")


if __name__ == "__main__":
    try:
        run()
    except Exception as exc:
        # Never print exception details: SDK errors may contain signed URLs or credentials.
        print(f"FAIL: S3 capability stage {STAGE}: {type(exc).__name__}", file=sys.stderr)
        if isinstance(exc, AssertionError):
            print(str(exc), file=sys.stderr)  # all assertion labels are static
        sys.exit(1)
