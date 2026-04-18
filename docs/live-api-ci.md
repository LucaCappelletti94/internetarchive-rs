# Live API CI

This repository has two CI layers:

- regular push and pull request checks in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)
- a scheduled live workflow in [`.github/workflows/live-daily.yml`](../.github/workflows/live-daily.yml)

The scheduled workflow runs the ignored tests in [`tests/live_smoke.rs`](../tests/live_smoke.rs).

Each daily run now exercises the full public client workflow surface against the real Internet Archive APIs:

- client construction through `new`, `with_auth`, `from_env`, and `builder`
- item creation through `create_item`
- reads through `get_item` and `get_item_by_str`
- search through `search`
- metadata writes through `apply_metadata_patch`, `apply_metadata_changes`, and `update_item_metadata`
- upload limit checks through `check_upload_limit`
- extra file uploads through `upload_file`
- download resolution and transfers through `resolve_download`, `download_bytes`, and `download_to_path`
- file deletion through `delete_file`
- high-level helpers through `publish_item` and `upsert_item`

## GitHub setup

Create a GitHub environment named `internetarchive-live` and add these secrets there:

- `INTERNET_ARCHIVE_ACCESS_KEY`
- `INTERNET_ARCHIVE_SECRET_KEY`

These are the LOW-auth S3 credentials from:

- `https://archive.org/account/s3.php`

## Operational notes

- The live workflow creates new tiny items on purpose. Keep the uploaded artifacts minimal.
- The workflow sets `skip_derive` on live uploads to reduce backend work.
- Internet Archive does not support deleting buckets through the S3-like API, so the live suite does not attempt cleanup of whole items after creation.
- The workflow is also exposed through `workflow_dispatch` so credentials or API behavior can be verified manually.
