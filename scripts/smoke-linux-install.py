#!/usr/bin/env python3
"""Install a real AppImage through the curl installer, open it on X11 without FUSE, remove it."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
image = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='mqx-install-smoke-') as directory:
    workspace = Path(directory)
    assets = workspace / 'assets'
    assets.mkdir()
    shutil.copyfile(image, assets / image.name)
    shutil.copyfile(ROOT / 'src-tauri/icons/128x128.png', assets / 'mqx.png')
    (assets / 'SHA256SUMS').write_text(''.join(
        f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n' for path in assets.iterdir()))
    (assets / 'release.json').write_text(json.dumps({'assets': [{
        'browser_download_url': f'https://github.com/hauke-gathmann/mqx/releases/download/fixture/{image.name}'
    }]}, indent=2))
    tools = workspace / 'tools'
    tools.mkdir()
    curl = tools / 'curl'
    curl.write_text('''#!/usr/bin/env python3
import sys, os, pathlib, shutil
args = sys.argv[1:]
url = next(a for a in args if a.startswith('https://'))
name = 'release.json' if 'api.github.com' in url else url.rsplit('/', 1)[1]
shutil.copyfile(pathlib.Path(os.environ['MQX_FIXTURES']) / name, args[args.index('-o') + 1])
''')
    curl.chmod(0o755)
    env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'],
               MQX_BIN=str(workspace / 'bin'), XDG_DATA_HOME=str(workspace / 'data'),
               XDG_CONFIG_HOME=str(workspace / 'config'), XDG_CACHE_HOME=str(workspace / 'cache'),
               MQX_FIXTURES=str(assets), APPIMAGE_EXTRACT_AND_RUN='1', LIBGL_ALWAYS_SOFTWARE='1')
    for _ in range(2):
        subprocess.run(['sh', str(ROOT / 'install.sh')], env=env, check=True, stdin=subprocess.DEVNULL)
    # Run inside a temporary desktop/session bus. A real titled window must appear.
    probe = workspace / 'probe.sh'
    probe.write_text('''#!/bin/sh
set -eu
"$MQX_BIN/mqx" > "$MQX_FIXTURES/launch.log" 2>&1 &
app_pid=$!
trap 'kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true' EXIT
for attempt in $(seq 1 30); do
  if ! kill -0 "$app_pid" 2>/dev/null; then cat "$MQX_FIXTURES/launch.log"; exit 1; fi
  if xwininfo -root -tree | grep -q '"mqx"'; then
    echo 'mqx opened a real X11 window without FUSE.'
    exit 0
  fi
  sleep 1
done
cat "$MQX_FIXTURES/launch.log"
echo 'mqx did not open a window.' >&2
exit 1
''')
    subprocess.run(['dbus-run-session', '--', 'xvfb-run', '-a', 'sh', str(probe)], env=env, check=True, timeout=60)
    subprocess.run(['sh', str(ROOT / 'install.sh'), '--uninstall'], env=env, check=True)
    assert not (workspace / 'bin/mqx').exists()
    assert not (workspace / 'data/mqx/app/mqx.AppImage').exists()
print('Real Linux installation, replacement, no-FUSE launch and uninstall checks passed.')
