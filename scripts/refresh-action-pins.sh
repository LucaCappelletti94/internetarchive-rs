#!/usr/bin/env bash
# Refreshes the dtolnay/rust-toolchain digests, which Dependabot cannot track because
# the branch selects the action variant and the only tag is a major alias.
# With --check the pins are compared rather than rewritten.
set -euo pipefail

check_only=false
[ "${1:-}" = "--check" ] && check_only=true

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workflows=("$repo_root"/.github/workflows/*.yml)
action="dtolnay/rust-toolchain"
stale=0

for branch in stable master nightly; do
  head="$(gh api "repos/${action}/commits/${branch}" -q .sha)"
  pinned="$(grep -hoE "uses: ${action}@[0-9a-f]{40} # ${branch}\$" "${workflows[@]}" \
    | head -1 | grep -oE '[0-9a-f]{40}' || true)"

  if [ -z "$pinned" ]; then
    echo "no pin found for ${action}@${branch}" >&2
    exit 1
  fi

  if [ "$pinned" = "$head" ]; then
    echo "${branch}: up to date (${head:0:12})"
    continue
  fi

  stale=1
  echo "${branch}: ${pinned:0:12} -> ${head:0:12}"
  if [ "$check_only" = false ]; then
    sed -i -E "s|uses: ${action}@[0-9a-f]{40} # ${branch}\$|uses: ${action}@${head} # ${branch}|" \
      "${workflows[@]}"
  fi
done

if [ "$check_only" = true ] && [ "$stale" -eq 1 ]; then
  echo "run scripts/refresh-action-pins.sh to update these pins" >&2
  exit 1
fi
