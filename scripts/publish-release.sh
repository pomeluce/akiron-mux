#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <tag> <notes-file> <artifacts-directory>" >&2
  exit 2
fi

release_tag=$1
notes_file=$2
artifacts_directory=$3

if [[ ! -f "$notes_file" ]]; then
  echo "release notes file does not exist: $notes_file" >&2
  exit 1
fi
if [[ ! -d "$artifacts_directory" ]]; then
  echo "artifacts directory does not exist: $artifacts_directory" >&2
  exit 1
fi

mapfile -d '' release_assets < <(find "$artifacts_directory" -type f -print0 | sort -z)
if [[ ${#release_assets[@]} -eq 0 ]]; then
  echo "no release assets found in: $artifacts_directory" >&2
  exit 1
fi

if gh release view "$release_tag" >/dev/null 2>&1; then
  gh release upload "$release_tag" "${release_assets[@]}" --clobber
else
  gh release create "$release_tag" \
    --title "$release_tag" \
    --notes-file "$notes_file" \
    "${release_assets[@]}"
fi
