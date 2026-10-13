#!/bin/sh
# Install verified release artifacts; never change Gatekeeper or quarantine settings.
set -eu
umask 022
REPO=${MQX_REPO:-hauke-gathmann/mqx}
VERSION=${MQX_VERSION:-latest}
MODE=install
fail() { echo "mqx: $*" >&2; exit 1; }
usage() {
  echo 'Usage: sh install.sh [--version vX.Y.Z] [--uninstall]'
  echo 'Optional: MQX_APP_DIR (macOS), MQX_BIN and XDG_DATA_HOME (Linux).'
}
while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) [ "$#" -ge 2 ] || fail '--version needs a tag'; VERSION=$2; shift 2 ;;
    --uninstall) MODE=uninstall; shift ;;
    --help|-h) usage; exit 0 ;;
    *) fail "Unknown option: $1" ;;
  esac
done
case "$REPO" in *[!A-Za-z0-9_./-]*|/*|*..*) fail 'Invalid repository' ;; esac
case "$VERSION" in latest|v[0-9]*) ;; *) fail 'Use latest or a version tag such as v0.2.0' ;; esac
case "$VERSION" in *[!A-Za-z0-9.+-]*) fail 'Invalid version tag' ;; esac
os=$(uname -s)
arch=$(uname -m)
case "$os-$arch" in
  Darwin-arm64|Darwin-aarch64) pattern='_aarch64\.dmg$' ;;
  Darwin-x86_64) pattern='_x64\.dmg$|_x86_64\.dmg$' ;;
  Linux-x86_64|Linux-amd64) pattern='_amd64\.AppImage$|_x86_64\.AppImage$|_x64\.AppImage$' ;;
  *) fail "Unsupported platform $os/$arch. See https://github.com/$REPO/releases" ;;
esac
# Use absolute paths only. Reject characters with special desktop-entry semantics.
check_path() {
  case "$1" in /*) ;; *) fail "Installation path must be absolute: $1" ;; esac
  case "$1" in *'
'*|*'"'*|*'`'*|*'$'*|*'%'*|*\\*) fail 'Unsupported character in installation path' ;; esac
}
if [ "$os" = Darwin ]; then
  appdir=${MQX_APP_DIR:-/Applications}
  if [ -z "${MQX_APP_DIR:-}" ] && [ ! -w "$appdir" ]; then appdir="$HOME/Applications"; fi
  check_path "$appdir"
  dest="$appdir/mqx.app"
else
  bindir=${MQX_BIN:-$HOME/.local/bin}
  data=${XDG_DATA_HOME:-$HOME/.local/share}
  check_path "$bindir"; check_path "$data"
  appdir="$data/mqx/app"
  dest="$appdir/mqx.AppImage"
  launcher="$bindir/mqx"
  desktop="$data/applications/dev.mqx.app.desktop"
  icon="$data/icons/hicolor/128x128/apps/dev.mqx.app.png"
fi
if [ "$MODE" = uninstall ]; then
  if [ "$os" = Darwin ]; then
    [ ! -e "$dest" ] || rm -rf "$dest"
  else
    # Remove only the installer-owned launcher and artifacts, never user data.
    if [ -f "$launcher" ] && grep -q '^# mqx installer launcher$' "$launcher"; then rm -f "$launcher"; fi
    rm -f "$dest" "$desktop" "$icon"
    rmdir "$appdir" 2>/dev/null || true
  fi
  echo 'mqx uninstalled. Profiles, passwords, recordings, and settings were preserved.'
  exit 0
fi
command -v curl >/dev/null 2>&1 || fail 'curl is required'
if command -v sha256sum >/dev/null 2>&1; then hash_tool=sha256sum
elif command -v shasum >/dev/null 2>&1; then hash_tool=shasum
else fail 'sha256sum or shasum is required'; fi
mkdir -p "$appdir"
# A directory lock serializes installs without depending on flock (absent on macOS).
lock="$appdir/.mqx-install-lock"
mkdir "$lock" 2>/dev/null || fail "Another installation is in progress ($lock)."
tmp=
mount=
stage=
backup=
cleanup() {
  if [ -n "$backup" ] && [ -e "$backup" ] && [ ! -e "$dest" ]; then
    if ! mv "$backup" "$dest"; then
      echo "Restore the previous app from $backup" >&2
      stage= # Preserve the recovery copy if restoration itself fails.
    fi
  fi
  if [ -n "$mount" ]; then hdiutil detach -quiet "$mount" 2>/dev/null || true; fi
  [ -z "$stage" ] || rm -rf "$stage"
  [ -z "$tmp" ] || rm -rf "$tmp"
  rmdir "$lock" 2>/dev/null || true
}
finish() { install_status=$?; cleanup; exit "$install_status"; }
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
tmp=$(mktemp -d)
fetch() {
  curl --fail --silent --show-error --location --retry 3 --connect-timeout 15 --max-time 300 \
    --proto '=https' --proto-redir '=https' --tlsv1.2 "$1" -o "$2"
}
api="https://api.github.com/repos/$REPO/releases"
if [ "$VERSION" = latest ]; then endpoint="$api/latest"; else endpoint="$api/tags/$VERSION"; fi
fetch "$endpoint" "$tmp/release.json" || fail 'Could not fetch the release. Check the version, network, GitHub API limit, and whether a release has been published.'
# GitHub's browser_download_url values are strings; only accept our repository's asset URLs.
urls=$(sed -n 's/.*"browser_download_url": *"\([^"]*\)".*/\1/p' "$tmp/release.json")
asset=$(printf '%s\n' "$urls" | grep -E "$pattern" || true)
[ "$(printf '%s\n' "$asset" | grep -c '^https://' || true)" = 1 ] || fail 'Release has no unique installer for this platform.'
case "$asset" in "https://github.com/$REPO/releases/download/"*) ;; *) fail 'Unexpected asset URL' ;; esac
filename=${asset##*/}
case "$filename" in *[!A-Za-z0-9._+-]*) fail 'Unexpected artifact filename' ;; esac
base=${asset%/*}
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail 'Release is missing its checksum manifest.'
verify() {
  name=$1
  expected=$(awk -v name="$name" '$2 == name { print $1 }' "$tmp/SHA256SUMS")
  [ "${#expected}" = 64 ] || fail "Missing or ambiguous checksum for $name"
  case "$expected" in *[!0-9a-f]*) fail 'Invalid checksum' ;; esac
  if [ "$hash_tool" = sha256sum ]; then actual=$(sha256sum "$tmp/$name" | awk '{print $1}')
  else actual=$(shasum -a 256 "$tmp/$name" | awk '{print $1}'); fi
  [ "$actual" = "$expected" ] || fail "Checksum mismatch for $name; existing installation preserved."
}
echo "Downloading ${filename}..."
fetch "$asset" "$tmp/$filename" || fail 'Download failed; existing installation preserved.'
verify "$filename"
if [ "$os" = Darwin ]; then
  for tool in hdiutil ditto codesign pgrep; do command -v "$tool" >/dev/null 2>&1 || fail "$tool is required"; done
  if pgrep -x mqx >/dev/null 2>&1; then fail 'Quit mqx before installing an update.'; fi
  mount="$tmp/mount"; mkdir "$mount"
  hdiutil attach -nobrowse -readonly -quiet -mountpoint "$mount" "$tmp/$filename"
  [ -d "$mount/mqx.app" ] || fail 'The disk image does not contain mqx.app.'
  codesign --verify --deep --strict "$mount/mqx.app" || fail 'App code signature verification failed.'
  stage=$(mktemp -d "$appdir/.mqx-stage.XXXXXX")
  ditto "$mount/mqx.app" "$stage/mqx.app"
  codesign --verify --deep --strict "$stage/mqx.app"
  backup="$stage/previous.app"
  if [ -e "$dest" ]; then mv "$dest" "$backup"; fi
  if ! mv "$stage/mqx.app" "$dest"; then
    fail 'Replacement failed; restoring the previous installation.'
  fi
  # Only remove the backup after the replacement succeeded.
  rm -rf "$backup"; backup=
  echo "Installed $dest"
  echo 'This app is ad-hoc signed, without Apple notarization. If macOS blocks launch, review it in System Settings → Privacy & Security → Open Anyway.'
  echo "Open it from Applications, or: open \"$dest\""
else
  fetch "$base/mqx.png" "$tmp/mqx.png" || fail 'Release is missing its desktop icon.'
  verify mqx.png
  mkdir -p "$bindir" "$(dirname "$desktop")" "$(dirname "$icon")"
  if [ -e "$launcher" ] && ! grep -q '^# mqx installer launcher$' "$launcher"; then
    # Migrate only the old installer's AppImage; refuse to overwrite arbitrary commands.
    magic=$(od -An -tx1 -N3 -j8 "$launcher" 2>/dev/null | tr -d ' \n')
    [ "$magic" = 414902 ] || fail "Refusing to overwrite an unrelated command: $launcher"
  fi
  stage=$(mktemp -d "$appdir/.mqx-stage.XXXXXX")
  cp "$tmp/$filename" "$stage/mqx.AppImage"; chmod 755 "$stage/mqx.AppImage"
  # Same-filesystem rename keeps a failed download/copy from damaging the old binary.
  mv -f "$stage/mqx.AppImage" "$dest"
  cat > "$stage/launcher" <<EOF
#!/bin/sh
# mqx installer launcher
# Extraction also works where FUSE is unavailable; Tauri still sees the AppImage path.
if [ ! -r /dev/fuse ] || [ ! -w /dev/fuse ]; then
  export APPIMAGE_EXTRACT_AND_RUN=1
fi
exec "$dest" "\$@"
EOF
  chmod 755 "$stage/launcher"
  # Destinations may live on different filesystems, so stage each file alongside its target.
  cp "$stage/launcher" "$launcher.new"; chmod 755 "$launcher.new"; mv -f "$launcher.new" "$launcher"
  cp "$tmp/mqx.png" "$icon.new"; mv -f "$icon.new" "$icon"
  cat > "$desktop.new" <<EOF
[Desktop Entry]
Type=Application
Name=mqx
Comment=Explore MQTT topics and messages
Exec="$launcher"
Icon=dev.mqx.app
Terminal=false
Categories=Development;Network;
StartupWMClass=mqx
EOF
  mv -f "$desktop.new" "$desktop"
  if command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database "$(dirname "$desktop")" 2>/dev/null || true; fi
  echo "Installed mqx. Open it from your application menu, or run: $launcher"
  echo 'If FUSE is unavailable, run with APPIMAGE_EXTRACT_AND_RUN=1.'
fi
