#!/usr/bin/env python3
"""Exercise the real DMG, hdiutil, codesign and replacement path; only HTTP is local."""
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
    digest = hashlib.sha256(image.read_bytes()).hexdigest()
    (assets / 'SHA256SUMS').write_text(f'{digest}  {image.name}\n')
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
    appdir = workspace / 'Applications'
    app = appdir / 'mqx.app'
    env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'],
               MQX_APP_DIR=str(appdir), MQX_FIXTURES=str(assets))
    for _ in range(2):
        subprocess.run(['sh', str(ROOT / 'install.sh')], env=env, check=True, stdin=subprocess.DEVNULL)
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    subprocess.run(['sh', str(ROOT / 'install.sh'), '--uninstall'], env=env, check=True)
    assert not app.exists(), 'Uninstall left the application behind'
print('Real macOS installation, replacement, signature and uninstall checks passed.')
