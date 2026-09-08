#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <tag> <artifacts-directory> <candidate-directory>" >&2
  exit 2
fi

release_tag=$1
artifacts_directory=$2
candidate_directory=$3
release_version=${release_tag#v}

if [[ "$release_tag" != "v$release_version" || -z "$release_version" ]]; then
  echo "release tag must start with v: $release_tag" >&2
  exit 1
fi
if [[ ! -d "$artifacts_directory" ]]; then
  echo "artifacts directory does not exist: $artifacts_directory" >&2
  exit 1
fi
if [[ -e "$candidate_directory" ]]; then
  echo "candidate directory already exists: $candidate_directory" >&2
  exit 1
fi

cli_name="AkironMux-${release_version}-linux-x86_64-cli.tar.gz"
desktop_name="AkironMux-${release_version}-linux-x86_64-desktop.deb"
cli_path=$(find "$artifacts_directory" -type f -name "$cli_name" -print -quit)
desktop_path=$(find "$artifacts_directory" -type f -name "$desktop_name" -print -quit)

if [[ -z "$cli_path" || -z "$desktop_path" ]]; then
  echo "Linux x86_64 CLI and desktop assets are required for Nix metadata" >&2
  exit 1
fi

mkdir -p "$candidate_directory/assets" "$candidate_directory/nix"
while IFS= read -r -d '' artifact; do
  install -Dm644 "$artifact" "$candidate_directory/assets/$(basename "$artifact")"
done < <(find "$artifacts_directory" -type f -print0)

(
  cd "$candidate_directory/assets"
  sha256sum ./* >../SHA256SUMS
)

cli_hash=$(sha256sum "$cli_path" | cut -d ' ' -f 1)
desktop_hash=$(sha256sum "$desktop_path" | cut -d ' ' -f 1)

printf '%s\n' \
  '{' \
  "  version = \"${release_version}\";" \
  '' \
  '  systems.x86_64-linux = {' \
  '    cli = {' \
  "      fileName = \"${cli_name}\";" \
  "      sha256 = \"${cli_hash}\";" \
  '    };' \
  '    desktop = {' \
  "      fileName = \"${desktop_name}\";" \
  "      sha256 = \"${desktop_hash}\";" \
  '    };' \
  '  };' \
  '}' \
  >"$candidate_directory/nix/release-assets.nix"

echo "release candidate prepared in: $candidate_directory"
