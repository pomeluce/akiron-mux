#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fixture_root=$(mktemp -d)
trap 'rm -rf "$fixture_root"' EXIT

mkdir -p "$fixture_root/artifacts/linux"
printf 'cli asset\n' >"$fixture_root/artifacts/linux/AkironMux-2.0.0-linux-x86_64-cli.tar.gz"
printf 'desktop asset\n' >"$fixture_root/artifacts/linux/AkironMux-2.0.0-linux-x86_64-desktop.deb"

bash "$repo_root/scripts/prepare-release-candidate.sh" \
  v2.0.0 \
  "$fixture_root/artifacts" \
  "$fixture_root/candidate"

grep -q 'version = "2.0.0";' "$fixture_root/candidate/nix/release-assets.nix"
grep -q 'AkironMux-2.0.0-linux-x86_64-cli.tar.gz' "$fixture_root/candidate/nix/release-assets.nix"
grep -q 'AkironMux-2.0.0-linux-x86_64-desktop.deb' "$fixture_root/candidate/nix/release-assets.nix"
(
  cd "$fixture_root/candidate/assets"
  sha256sum -c ../SHA256SUMS
)

echo 'release candidate regression test passed'
