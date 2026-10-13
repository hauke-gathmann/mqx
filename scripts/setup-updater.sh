#!/bin/sh
# Creates a local key only if absent, and uploads it to the repository's Actions secrets.
# Never print private key material. Run from the repository root.
set -eu
umask 077
keydir=${MQX_SIGNING_DIR:-$HOME/.config/mqx-release}
mkdir -p "$keydir"
chmod 700 "$keydir"
if [ ! -f "$keydir/updater.key" ]; then
  releases=$(gh release list --repo hauke-gathmann/mqx --limit 1 --json tagName --jq length)
  [ "$releases" = 0 ] || {
    echo 'A release already exists. Recover the original updater key instead of creating a replacement.' >&2
    exit 1
  }
  npm run tauri signer generate -- --ci -p '' -w "$keydir/updater.key" > "$keydir/generation.log" 2>&1
fi
chmod 600 "$keydir/updater.key"
test -f "$keydir/updater.key.pub" || { echo 'Missing public key; recover the matching pair before continuing.' >&2; exit 1; }
node --input-type=module - "$keydir/updater.key.pub" <<'JS'
import { readFileSync, writeFileSync } from 'node:fs';
const publicKey = readFileSync(process.argv[2], 'utf8').trim();
const path = 'src-tauri/tauri.conf.json';
const config = JSON.parse(readFileSync(path, 'utf8'));
config.plugins.updater.pubkey = publicKey;
writeFileSync(path, JSON.stringify(config, null, 2) + '\n');
JS
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo hauke-gathmann/mqx < "$keydir/updater.key"
echo "Updater key configured. Back up $keydir securely; it is required for future updates."
