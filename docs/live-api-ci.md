# Live API CI

This repository has two CI layers:

- regular push and pull request checks in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)
- a scheduled live workflow in [`.github/workflows/live-daily.yml`](../.github/workflows/live-daily.yml)

The scheduled workflow runs the live tests in [`tests/live_smoke.rs`](../tests/live_smoke.rs).

Each daily run now exercises the full public client workflow surface against the real Internet Archive APIs:

- client construction through `new`, `with_auth`, `from_env`, and `builder`
- item creation through `create_item`
- reads through `get_item` and `get_item_by_str`
- search through `search`
- metadata writes through `apply_metadata_patch`, `apply_metadata_changes`, and `update_item_metadata`
- upload limit checks through `check_upload_limit`
- extra file uploads through `upload_file`
- download resolution and transfers through `resolve_download`, `download_bytes`, and `download_to_path`
- progress-bar streaming through `create_item_with_progress`, `upload_file_with_progress`, `download_bytes_with_progress`, and `download_to_path_with_progress` (under the `indicatif` feature)
- file deletion through `delete_file`
- item darkening through `make_dark`
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
- Internet Archive does not support deleting buckets through the S3-like API, but LOW-auth users may submit a `make_dark.php` task against items they uploaded. Each live test wraps the item it creates in a `LiveItemGuard` whose Drop submits that task with a short retry-on-failure loop, hiding the item from search, `/details/`, and the metadata API. The Python probe script issues the same cleanup at the end of its run. The IA catalog usually processes the queued darken task within seconds; the item's metadata stub remains with `is_dark: true` but is no longer publicly accessible.
- The live tests use `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]` so the guard's Drop can call `tokio::task::block_in_place` + `Handle::current().block_on` for synchronous cleanup. The CI live job invokes `cargo test --test live_smoke --all-features --locked -- --nocapture --test-threads=1`; the `--test-threads=1` flag also keeps the cleanup tasks well below IA's per-user rate limit.
- The workflow is also exposed through `workflow_dispatch` so credentials or API behavior can be verified manually.
- The live tests are regular tests. They always run, and they return early only when the required credentials are absent.
