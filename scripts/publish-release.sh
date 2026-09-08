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

release_version=${release_tag#v}
asset_prefix="AkironMux-${release_version}-"
for release_asset in "${release_assets[@]}"; do
  asset_name=$(basename "$release_asset")
  case "$asset_name" in
    "${asset_prefix}linux-x86_64-cli.tar.gz" | \
      "${asset_prefix}linux-x86_64-cli.deb" | \
      "${asset_prefix}linux-x86_64-cli.rpm" | \
      "${asset_prefix}linux-x86_64-desktop.deb" | \
      "${asset_prefix}linux-x86_64-desktop.AppImage" | \
      "${asset_prefix}macos-arm64-cli.tar.gz" | \
      "${asset_prefix}macos-arm64-desktop.dmg" | \
      "${asset_prefix}windows-x86_64-cli.zip" | \
      "${asset_prefix}windows-x86_64-desktop-setup.exe") ;;
    *)
      echo "unexpected release asset name: $asset_name" >&2
      exit 1
      ;;
  esac
done

if gh release view "$release_tag" >/dev/null 2>&1; then
  existing_assets_directory=$(mktemp -d)
  trap 'rm -rf "$existing_assets_directory"' EXIT
  gh release download "$release_tag" --dir "$existing_assets_directory"

  for release_asset in "${release_assets[@]}"; do
    asset_name=$(basename "$release_asset")
    existing_asset="$existing_assets_directory/$asset_name"
    if [[ ! -f "$existing_asset" ]]; then
      echo "existing release is missing asset: $asset_name" >&2
      exit 1
    fi
    if ! cmp -s "$release_asset" "$existing_asset"; then
      echo "existing release asset differs and cannot be overwritten: $asset_name" >&2
      exit 1
    fi
  done

  mapfile -t existing_asset_names < <(find "$existing_assets_directory" -type f -printf '%f\n' | sort)
  mapfile -t release_asset_names < <(printf '%s\n' "${release_assets[@]##*/}" | sort)
  if [[ "${existing_asset_names[*]}" != "${release_asset_names[*]}" ]]; then
    echo "existing release contains a different asset set and cannot be overwritten" >&2
    exit 1
  fi

  echo "release $release_tag already contains the same immutable assets"
  exit 0
fi

gh release create "$release_tag" \
  --title "$release_tag" \
  --notes-file "$notes_file" \
  "${release_assets[@]}"
