#!/bin/sh
# Install mqx from the latest GitHub Release.
# macOS → /Applications/mqx.app
# Linux x86_64 → ~/.local/bin/mqx (AppImage)
set -eu

REPO="${MQX_REPO:-hauke-gathmann/mqx}"
API="https://api.github.com/repos/${REPO}/releases"
RELEASES="https://github.com/${REPO}/releases"

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "mqx install: need '$1' on PATH" >&2
    exit 1
  fi
}

http_get() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --proto '=https' --tlsv1.2 "$@"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$@"
  else
    echo "mqx install: need curl or wget" >&2
    exit 1
  fi
}

http_download() {
  dest=$1
  url=$2
  if command -v curl >/dev/null 2>&1; then
    curl -fL --proto '=https' --tlsv1.2 --progress-bar -o "$dest" "$url"
  else
    wget -O "$dest" "$url"
  fi
}

os=$(uname -s)
arch=$(uname -m)

case "$os" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  MINGW*|MSYS*|CYGWIN*)
    echo "Windows: download the installer from ${RELEASES}/latest" >&2
    echo "Prefer mqx_*_x64-setup.exe (NSIS). MSI is on the same page." >&2
    exit 1
    ;;
  *)
    echo "mqx install: unsupported OS $(uname -s)" >&2
    exit 1
    ;;
esac

case "$arch" in
  arm64|aarch64) arch=aarch64 ;;
  x86_64|amd64) arch=x86_64 ;;
  *)
    echo "mqx install: unsupported architecture $arch" >&2
    exit 1
    ;;
esac

if [ "$os" = linux ] && [ "$arch" != x86_64 ]; then
  echo "mqx install: Linux builds are x86_64 only (no aarch64 AppImage yet)." >&2
  echo "Download whatever exists from ${RELEASES}/latest" >&2
  exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Fetching latest mqx release…"
json=$(http_get "${API}/latest") || {
  echo "mqx install: no published GitHub Release yet." >&2
  echo "Publish the draft at ${RELEASES} and retry." >&2
  exit 1
}

urls=$(printf '%s\n' "$json" | tr -d '\r' | grep -o 'https://github.com/[^"]*/releases/download/[^"]*' || true)
if [ -z "$urls" ]; then
  echo "mqx install: latest release has no downloadable assets." >&2
  echo "See ${RELEASES}/latest" >&2
  exit 1
fi

pick() {
  printf '%s\n' "$urls" | grep -E "$1" | grep -vE '\.(sig|json)$' | grep -v '\.tar\.gz' | head -n 1 || true
}

asset=
case "$os-$arch" in
  macos-aarch64) asset=$(pick 'aarch64\.dmg$') ;;
  macos-x86_64) asset=$(pick '_x64\.dmg$|_x86_64\.dmg$') ;;
  linux-x86_64) asset=$(pick 'amd64\.AppImage$|_x86_64\.AppImage$|_x64\.AppImage$') ;;
esac

if [ -z "$asset" ]; then
  echo "mqx install: no matching asset for $os $arch in the latest release." >&2
  echo "Available downloads:" >&2
  printf '%s\n' "$urls" >&2
  echo "See ${RELEASES}/latest" >&2
  exit 1
fi

filename=$(printf '%s\n' "$asset" | sed 's|.*/||')
echo "Downloading $filename"

if [ "$os" = macos ]; then
  need hdiutil
  need ditto
  dmg="$tmp/$filename"
  http_download "$dmg" "$asset"
  mount="$tmp/mnt"
  mkdir -p "$mount"
  hdiutil attach -nobrowse -quiet -mountpoint "$mount" "$dmg"
  # Detach the dmg even if copy fails; tmp cleanup is the EXIT trap.
  detach() { hdiutil detach -quiet "$mount" 2>/dev/null || true; }
  trap 'detach; rm -rf "$tmp"' EXIT
  app=$(find "$mount" -maxdepth 1 -name '*.app' -print | head -n 1)
  if [ -z "$app" ]; then
    echo "mqx install: no .app in $filename" >&2
    exit 1
  fi
  appname=$(basename "$app")
  dest="/Applications/$appname"
  echo "Installing $appname → $dest"
  if [ -d "$dest" ]; then
    rm -rf "$dest" 2>/dev/null || sudo rm -rf "$dest"
  fi
  if ! ditto "$app" "$dest" 2>/dev/null; then
    sudo ditto "$app" "$dest"
  fi
  xattr -cr "$dest" 2>/dev/null || true
  detach
  trap 'rm -rf "$tmp"' EXIT
  echo "Installed $dest"
  echo "Open it from Launchpad, or: open \"$dest\""
  echo "If macOS blocks it (unsigned build), right-click the app → Open."
else
  dest="${MQX_BIN:-$HOME/.local/bin}/mqx"
  mkdir -p "$(dirname "$dest")"
  http_download "$tmp/$filename" "$asset"
  chmod +x "$tmp/$filename"
  mv "$tmp/$filename" "$dest"
  echo "Installed $dest"
  case ":$PATH:" in
    *:"$(dirname "$dest")":*)
      echo "Run: mqx"
      ;;
    *)
      echo "Add $(dirname "$dest") to PATH, then run: mqx"
      ;;
  esac
fi
