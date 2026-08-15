#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fixture_root=$(mktemp -d)
trap 'rm -rf "$fixture_root"' EXIT

mkdir -p "$fixture_root/bin" "$fixture_root/artifacts/appimage" "$fixture_root/artifacts/deb"
touch "$fixture_root/artifacts/appimage/AkironMux.AppImage"
touch "$fixture_root/artifacts/deb/akiron-mux.deb"
touch "$fixture_root/artifacts/akmux.tar.gz"
printf 'Release notes\n' >"$fixture_root/CHANGELOG.md"

cat >"$fixture_root/bin/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "$1 $2" == "release view" ]]; then
  [[ "${FAKE_RELEASE_EXISTS:-false}" == "true" ]]
  exit
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
  v1.11.0 \
  "$fixture_root/CHANGELOG.md" \
  "$fixture_root/artifacts"

grep -q '^release create ' "$GH_CALL_LOG"
grep -q 'AkironMux.AppImage' "$GH_CALL_LOG"
grep -q 'akiron-mux.deb' "$GH_CALL_LOG"
grep -q 'akmux.tar.gz' "$GH_CALL_LOG"

: >"$GH_CALL_LOG"
FAKE_RELEASE_EXISTS=true bash "$repo_root/scripts/publish-release.sh" \
  v1.11.0 \
  "$fixture_root/CHANGELOG.md" \
  "$fixture_root/artifacts"

grep -q '^release upload .* --clobber' "$GH_CALL_LOG"
if grep -q '^release create ' "$GH_CALL_LOG"; then
  echo 'existing releases must not be recreated' >&2
  exit 1
fi

echo 'release publish regression test passed'
