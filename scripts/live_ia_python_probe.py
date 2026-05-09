#!/usr/bin/env python3
"""Create one tiny IA item through the official Python client.

This script is intentionally used only by the credentialed live CI workflow.
It answers whether the same account/secrets can create an Internet Archive
test item through the maintained upstream client before the Rust live suite
tries the same operation.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
import tempfile
import time
from pathlib import Path

import internetarchive


ACCESS_KEY_ENV = "INTERNET_ARCHIVE_ACCESS_KEY"
SECRET_KEY_ENV = "INTERNET_ARCHIVE_SECRET_KEY"
TEST_COLLECTION = "test_collection"


def required_env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f"Missing {name}")
    return value


def live_identifier() -> str:
    seed = ":".join(
        [
            os.environ.get("GITHUB_RUN_ID", "local"),
            os.environ.get("GITHUB_RUN_ATTEMPT", "0"),
            str(os.getpid()),
            str(time.time_ns()),
        ]
    )
    digest = hashlib.sha256(seed.encode("utf-8")).hexdigest()[:12]
    return f"iarustclientprobe{int(time.time())}{digest}"


def sanitized_headers(headers: dict[str, str]) -> dict[str, str]:
    return {
        key: value
        for key, value in headers.items()
        if key.lower() != "authorization"
    }


def main() -> int:
    access_key = required_env(ACCESS_KEY_ENV)
    secret_key = required_env(SECRET_KEY_ENV)
    identifier = live_identifier()
    metadata = {
        "collection": TEST_COLLECTION,
        "description": "internetarchive-rs official client live probe",
        "language": "eng",
        "mediatype": "texts",
        "title": f"internetarchive-rs official client probe {identifier}",
    }

    with tempfile.TemporaryDirectory() as directory:
        artifact = Path(directory) / "official-client-probe.txt"
        artifact.write_text("internetarchive-rs official client probe\n", encoding="utf-8")

        debug_requests = internetarchive.upload(
            identifier,
            str(artifact),
            metadata=metadata,
            access_key=access_key,
            secret_key=secret_key,
            queue_derive=True,
            debug=True,
            validate_identifier=True,
            request_kwargs={"timeout": 120},
        )
        for request in debug_requests:
            print(f"official_client_probe_identifier={identifier}")
            print(f"official_client_probe_url={request.url}")
            print(
                "official_client_probe_headers="
                + json.dumps(sanitized_headers(dict(request.headers)), sort_keys=True)
            )

        try:
            responses = internetarchive.upload(
                identifier,
                str(artifact),
                metadata=metadata,
                access_key=access_key,
                secret_key=secret_key,
                queue_derive=True,
                validate_identifier=True,
                request_kwargs={"timeout": 120},
            )
        except Exception as error:  # noqa: BLE001 - CI diagnostic should preserve upstream errors.
            print(
                f"official_client_probe_failed={type(error).__name__}: {error}",
                file=sys.stderr,
            )
            response = getattr(error, "response", None)
            if response is not None:
                print(f"official_client_probe_status={response.status_code}", file=sys.stderr)
                print(
                    f"official_client_probe_body={response.text[:2000]}",
                    file=sys.stderr,
                )
            raise

    statuses = [getattr(response, "status_code", None) for response in responses]
    print(f"official_client_probe_statuses={statuses}")
    print(f"official_client_probe_created={identifier}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
