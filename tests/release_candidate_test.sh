#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fixture_root=$(mktemp -d)
trap 'rm -rf "$fixture_root"' EXIT

mkdir -p "$fixture_root/artifacts/linux"
printf 'cli asset\n' >"$fixture_root/artifacts/linux/AkironMux-2.0.0-linux-x86_64-cli.tar.gz"
printf 'desktop asset\n' >"$fixture_root/artifacts/linux/AkironMux-2.0.0-linux-x86_64-desktop.deb"

export SOURCE_COMMIT=0123456789abcdef0123456789abcdef01234567
bash "$repo_root/scripts/prepare-release-candidate.sh" \
  v2.0.0 \
  "$fixture_root/artifacts" \
  "$fixture_root/candidate"

grep -q 'version = "2.0.0";' "$fixture_root/candidate/nix/release-assets.nix"
grep -q 'AkironMux-2.0.0-linux-x86_64-cli.tar.gz' "$fixture_root/candidate/nix/release-assets.nix"
grep -q 'AkironMux-2.0.0-linux-x86_64-desktop.deb' "$fixture_root/candidate/nix/release-assets.nix"
test "$(cat "$fixture_root/candidate/source-commit")" = "$SOURCE_COMMIT"
cmp "$fixture_root/candidate/nix/release-assets.nix" "$fixture_root/candidate/assets/AkironMux-2.0.0-nix-release-assets.nix"
(
  cd "$fixture_root/candidate/assets"
  sha256sum -c ../SHA256SUMS
)

node "$repo_root/scripts/sync-release-metadata.mjs" v2.0.0 "$fixture_root/candidate" "$fixture_root/release-assets.nix"
cmp "$fixture_root/release-assets.nix" "$fixture_root/candidate/nix/release-assets.nix"
node "$repo_root/scripts/sync-release-metadata.mjs" v2.0.0 "$fixture_root/candidate" "$fixture_root/release-assets.nix"
sed 's/version = "2.0.0"/version = "3.0.0"/' "$fixture_root/release-assets.nix" >"$fixture_root/newer.nix"
node "$repo_root/scripts/sync-release-metadata.mjs" v2.0.0 "$fixture_root/candidate" "$fixture_root/newer.nix"
grep -q 'version = "3.0.0"' "$fixture_root/newer.nix"
if node "$repo_root/scripts/sync-release-metadata.mjs" v1.0.0 "$fixture_root/candidate" "$fixture_root/release-assets.nix" >/dev/null 2>&1; then
  echo 'metadata for a different tag must be rejected' >&2
  exit 1
fi
cp "$fixture_root/release-assets.nix" "$fixture_root/conflict.nix"
printf '# conflicting metadata\n' >>"$fixture_root/conflict.nix"
if node "$repo_root/scripts/sync-release-metadata.mjs" v2.0.0 "$fixture_root/candidate" "$fixture_root/conflict.nix" >/dev/null 2>&1; then
  echo 'same-version metadata conflicts must be rejected' >&2
  exit 1
fi
printf 'tampered asset\n' >>"$fixture_root/candidate/assets/AkironMux-2.0.0-linux-x86_64-cli.tar.gz"
if node "$repo_root/scripts/sync-release-metadata.mjs" v2.0.0 "$fixture_root/candidate" "$fixture_root/release-assets.nix" >/dev/null 2>&1; then
  echo 'metadata for altered release assets must be rejected' >&2
  exit 1
fi
if SOURCE_COMMIT=invalid bash "$repo_root/scripts/prepare-release-candidate.sh" v2.0.0 "$fixture_root/artifacts" "$fixture_root/invalid-candidate" >/dev/null 2>&1; then
  echo 'candidates without a valid source commit must be rejected' >&2
  exit 1
fi
test ! -e "$fixture_root/invalid-candidate"

echo 'release candidate regression test passed'
