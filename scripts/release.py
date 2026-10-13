#!/usr/bin/env python3
"""Release metadata validation, artifact verification, and checksum generation."""
import argparse
import base64
import hashlib
import json
import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def version_info(tag=None):
    package = json.loads((ROOT / 'package.json').read_text())
    config = json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text())
    cargo = tomllib.loads((ROOT / 'src-tauri/Cargo.toml').read_text())
    npm_lock = json.loads((ROOT / 'package-lock.json').read_text())
    cargo_lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
    locked_version = next(p['version'] for p in cargo_lock['package'] if p['name'] == 'mqx')
    version = package['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', version):
        raise ValueError('Application version must be a release or prerelease SemVer')
    if not all(v == version for v in (config['version'], cargo['package']['version'],
                                     npm_lock['version'], npm_lock['packages']['']['version'], locked_version)):
        raise ValueError('Application versions differ between manifests and lockfiles')
    if tag is not None and tag != 'v' + version:
        raise ValueError(f'Tag {tag!r} must equal v{version}')
    return version


def checksums(directory):
    files = sorted(p for p in directory.iterdir() if p.is_file() and p.name != 'SHA256SUMS')
    lines = []
    for path in files:
        digest = hashlib.sha256()
        with path.open('rb') as source:
            for block in iter(lambda: source.read(1024 * 1024), b''):
                digest.update(block)
        lines.append(f'{digest.hexdigest()}  {path.name}\n')
    (directory / 'SHA256SUMS').write_text(''.join(lines))


def verify(directory, version):
    files = {p.name for p in directory.iterdir() if p.is_file()}
    required = [
        rf'mqx_{re.escape(version)}_aarch64\.dmg',
        rf'mqx_{re.escape(version)}_x64\.dmg',
        rf'mqx_{re.escape(version)}_amd64\.AppImage',
        rf'mqx_{re.escape(version)}_amd64\.deb',
        rf'mqx-{re.escape(version)}-.*\.x86_64\.rpm',
        rf'mqx_{re.escape(version)}_x64-setup\.exe',
        rf'mqx_{re.escape(version)}_x64_en-US\.msi',
        r'mqx\.png', r'install\.sh', r'LICENSE', r'THIRD_PARTY_NOTICES\.md', r'latest\.json',
    ]
    for pattern in required:
        matches = [name for name in files if re.fullmatch(pattern, name)]
        if len(matches) != 1:
            raise ValueError(f'Expected one artifact matching {pattern}, got {matches}')
    metadata = json.loads((directory / 'latest.json').read_text())
    if metadata['version'].removeprefix('v') != version:
        raise ValueError('Updater metadata version differs from application version')
    for platform in ('darwin-aarch64', 'darwin-x86_64', 'linux-x86_64', 'windows-x86_64'):
        # tauri-action v1 may also emit installer-specific keys; require the generic fallback.
        item = metadata['platforms'][platform]
        url = item['url']
        prefix = f'https://github.com/hauke-gathmann/mqx/releases/download/v{version}/'
        if not url.startswith(prefix):
            raise ValueError(f'Unexpected update URL: {url}')
        name = url[len(prefix):]
        if name not in files or not (directory / (name + '.sig')).is_file():
            raise ValueError(f'Missing updater artifact/signature for {platform}: {name}')
        if (directory / (name + '.sig')).read_text().strip() != item['signature'].strip():
            raise ValueError(f'Updater signature metadata mismatch for {platform}')
        base64.b64decode(item['signature'], validate=True)
    print(f'Complete release artifact set verified for v{version}')


def notes(version):
    changelog = (ROOT / 'CHANGELOG.md').read_text()
    match = re.search(rf'^## {re.escape(version)}\s*\n(.*?)(?=^## |\Z)', changelog, re.M | re.S)
    if not match:
        raise ValueError(f'CHANGELOG.md needs an entry for {version}')
    return f'''mqx {version}

{match.group(1).strip()}

Install on macOS (Apple Silicon or Intel) or Linux x86_64:

```sh
curl -fsSL https://raw.githubusercontent.com/hauke-gathmann/mqx/v{version}/install.sh | sh -s -- --version v{version}
```

macOS builds are ad-hoc signed, without an Apple Developer ID or notarization.
macOS may require approval in System Settings → Privacy & Security.
Linux downloads include AppImage, DEB, and RPM; Linux ARM64 is not yet supported.
Windows installers are unsigned. See README.md for requirements and troubleshooting.
The installer verifies SHA-256 checksums. In-app updates verify a separate updater signature.
'''


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=['version', 'notes', 'verify', 'checksums'])
    parser.add_argument('--tag')
    parser.add_argument('--directory', type=Path)
    args = parser.parse_args()
    version = version_info(args.tag)
    if args.command == 'version':
        print(version)
    elif args.command == 'notes':
        print(notes(version))
    elif args.command == 'verify':
        verify(args.directory, version)
    else:
        checksums(args.directory)


if __name__ == '__main__':
    main()
