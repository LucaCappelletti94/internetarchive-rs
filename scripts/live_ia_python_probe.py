#!/usr/bin/env python3
"""Create one tiny IA item through the official Python client.

This script is intentionally used only by the credentialed live CI workflow.
It answers whether the same account/secrets can create an Internet Archive
test item through the maintained upstream client before the Rust live suite
tries the same operation.
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

import internetarchive
from requests import HTTPError


ACCESS_KEY_ENV = "INTERNET_ARCHIVE_ACCESS_KEY"
SECRET_KEY_ENV = "INTERNET_ARCHIVE_SECRET_KEY"
TEST_COLLECTION = "test_collection"
LIVE_IDENTIFIER_PREFIX = "internetarchiversprobe"
BUCKET_PERMISSION_HINT = "this user may lack the special permission"


def required_env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f"Missing {name}")
    return value


def live_identifier() -> str:
    timestamp = int(time.time()) % 10_000_000_000
    process = os.getpid() % 10_000
    return f"{LIVE_IDENTIFIER_PREFIX}{timestamp:010d}{process:04d}"


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
            response_text = ""
            if response is not None:
                response_text = response.text
                print(f"official_client_probe_status={response.status_code}", file=sys.stderr)
                print(
                    f"official_client_probe_body={response_text[:2000]}",
                    file=sys.stderr,
                )
            if is_account_identifier_permission_failure(error, response_text):
                print(
                    "official_client_probe_skip_reason="
                    "account cannot create generated live-test identifiers",
                    file=sys.stderr,
                )
                return 77
            raise

    statuses = [getattr(response, "status_code", None) for response in responses]
    print(f"official_client_probe_statuses={statuses}")
    print(f"official_client_probe_created={identifier}")

    cleanup_status = make_dark(identifier, access_key, secret_key)
    print(f"official_client_probe_cleanup={cleanup_status}")
    return 0


def make_dark(identifier: str, access_key: str, secret_key: str) -> str:
    """Submit a make_dark.php task so the probe item does not leak publicly."""
    payload = json.dumps(
        {
            "identifier": identifier,
            "cmd": "make_dark.php",
            "args": {"comment": "live CI probe cleanup"},
        }
    ).encode()
    request = urllib.request.Request(
        "https://archive.org/services/tasks.php",
        data=payload,
        method="POST",
        headers={
            "Authorization": f"LOW {access_key}:{secret_key}",
            "Content-Type": "application/json",
            "Accept": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            body = json.loads(response.read())
    except urllib.error.HTTPError as error:
        return f"http_error_{error.code}"
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
        return f"transport_error_{type(error).__name__}"

    if body.get("success"):
        task_id = body.get("value", {}).get("task_id")
        return f"queued_task_id={task_id}"
    return f"task_rejected={body.get('error', 'unknown')}"


def is_account_identifier_permission_failure(error: Exception, response_text: str) -> bool:
    if not isinstance(error, HTTPError):
        return False
    response = getattr(error, "response", None)
    if response is None or response.status_code != 400:
        return False
    return "InvalidBucketName" in response_text and BUCKET_PERMISSION_HINT in response_text


if __name__ == "__main__":
    raise SystemExit(main())
