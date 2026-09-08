#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fixture_root=$(mktemp -d)
trap 'rm -rf "$fixture_root"' EXIT

mkdir -p "$fixture_root/bin" "$fixture_root/artifacts/linux" "$fixture_root/artifacts/windows"
touch "$fixture_root/artifacts/linux/AkironMux-1.12.0-linux-x86_64-cli.tar.gz"
touch "$fixture_root/artifacts/linux/AkironMux-1.12.0-linux-x86_64-desktop.AppImage"
touch "$fixture_root/artifacts/windows/AkironMux-1.12.0-windows-x86_64-desktop-setup.exe"
printf 'Release notes\n' >"$fixture_root/CHANGELOG.md"

cat >"$fixture_root/bin/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "$1 $2" == "release view" ]]; then
  [[ "${FAKE_RELEASE_EXISTS:-false}" == "true" ]]
  exit
fi

if [[ "$1 $2" == "release download" ]]; then
  shift 2
  while [[ $# -gt 0 ]]; do
    if [[ "$1" == "--dir" ]]; then
      cp "$FAKE_EXISTING_ASSETS"/* "$2"/
      exit
    fi
    shift
  done
fi

printf '%s' "$1 $2" >>"$GH_CALL_LOG"
shift 2
for argument in "$@"; do
  if [[ -d "$argument" ]]; then
    printf 'directory passed as release asset: %s\n' "$argument" >&2
    exit 1
  fi
  printf ' %q' "$argument" >>"$GH_CALL_LOG"
done
printf '\n' >>"$GH_CALL_LOG"
EOF
chmod +x "$fixture_root/bin/gh"

export PATH="$fixture_root/bin:$PATH"
export GH_CALL_LOG="$fixture_root/gh-calls.log"

FAKE_RELEASE_EXISTS=false bash "$repo_root/scripts/publish-release.sh" \
  v1.12.0 \
  "$fixture_root/CHANGELOG.md" \
  "$fixture_root/artifacts"

grep -q '^release create ' "$GH_CALL_LOG"
grep -q 'AkironMux-1.12.0-linux-x86_64-cli.tar.gz' "$GH_CALL_LOG"
grep -q 'AkironMux-1.12.0-linux-x86_64-desktop.AppImage' "$GH_CALL_LOG"
grep -q 'AkironMux-1.12.0-windows-x86_64-desktop-setup.exe' "$GH_CALL_LOG"

: >"$GH_CALL_LOG"
mkdir -p "$fixture_root/existing-assets"
cp "$fixture_root/artifacts/linux/"* "$fixture_root/existing-assets/"
cp "$fixture_root/artifacts/windows/"* "$fixture_root/existing-assets/"
export FAKE_EXISTING_ASSETS="$fixture_root/existing-assets"
FAKE_RELEASE_EXISTS=true bash "$repo_root/scripts/publish-release.sh" \
  v1.12.0 \
  "$fixture_root/CHANGELOG.md" \
  "$fixture_root/artifacts"

if grep -q '^release upload ' "$GH_CALL_LOG"; then
  echo 'existing release assets must not be uploaded again' >&2
  exit 1
fi
if grep -q '^release create ' "$GH_CALL_LOG"; then
  echo 'existing releases must not be recreated' >&2
  exit 1
fi

printf 'different\n' >"$fixture_root/artifacts/linux/AkironMux-1.12.0-linux-x86_64-cli.tar.gz"
if FAKE_RELEASE_EXISTS=true bash "$repo_root/scripts/publish-release.sh" \
  v1.12.0 \
  "$fixture_root/CHANGELOG.md" \
  "$fixture_root/artifacts"; then
  echo 'changed assets on an existing release must be rejected' >&2
  exit 1
fi

touch "$fixture_root/artifacts/AkironMux.AppImage"
if FAKE_RELEASE_EXISTS=false bash "$repo_root/scripts/publish-release.sh" \
  v1.12.0 \
  "$fixture_root/CHANGELOG.md" \
  "$fixture_root/artifacts"; then
  echo 'unexpected asset names must be rejected' >&2
  exit 1
fi

echo 'release publish regression test passed'
