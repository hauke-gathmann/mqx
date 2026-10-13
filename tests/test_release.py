import importlib.util
import base64
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('release', Path(__file__).resolve().parents[1] / 'scripts/release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def test_tag_must_match_application(self):
        version = release.version_info()
        self.assertEqual(release.version_info('v' + version), version)
        with self.assertRaises(ValueError):
            release.version_info('v99.99.99')

    def test_incomplete_release_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                release.verify(Path(directory), release.version_info())

    def test_checksums_are_repeatable_and_exclude_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / 'artifact').write_text('hello')
            release.checksums(path)
            first = (path / 'SHA256SUMS').read_text()
            release.checksums(path)
            self.assertEqual(first, (path / 'SHA256SUMS').read_text())
            self.assertEqual(len(first.splitlines()), 1)
            self.assertIn('2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824', first)

    def test_complete_release_and_invalid_update_metadata(self):
        version = release.version_info()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            names = [f'mqx_{version}_aarch64.dmg', f'mqx_{version}_x64.dmg',
                     f'mqx_{version}_amd64.AppImage', f'mqx_{version}_amd64.deb',
                     f'mqx-{version}-1.x86_64.rpm', f'mqx_{version}_x64-setup.exe',
                     f'mqx_{version}_x64_en-US.msi', 'mqx.png', 'install.sh',
                     'LICENSE', 'THIRD_PARTY_NOTICES.md']
            for name in names:
                (path / name).write_bytes(b'artifact')
            metadata = {'version': version, 'platforms': {}}
            for platform, name in [('darwin-aarch64', 'mqx_aarch64.app.tar.gz'),
                                   ('darwin-x86_64', 'mqx_x64.app.tar.gz'),
                                   ('linux-x86_64', f'mqx_{version}_amd64.AppImage'),
                                   ('windows-x86_64', f'mqx_{version}_x64-setup.exe')]:
                (path / name).write_bytes(b'updater artifact')
                signature = base64.b64encode(b'fixture signature').decode()
                (path / (name + '.sig')).write_text(signature)
                metadata['platforms'][platform] = {
                    'url': f'https://github.com/hauke-gathmann/mqx/releases/download/v{version}/{name}',
                    'signature': signature,
                }
            def save():
                (path / 'latest.json').write_text(json.dumps(metadata))
            save()
            release.verify(path, version)
            item = metadata['platforms']['darwin-aarch64']
            correct_url = item['url']
            item['url'] = correct_url.replace('hauke-gathmann', 'another-owner')
            save()
            with self.assertRaisesRegex(ValueError, 'Unexpected update URL'):
                release.verify(path, version)
            item['url'] = correct_url
            item['signature'] = base64.b64encode(b'wrong signature').decode()
            save()
            with self.assertRaisesRegex(ValueError, 'signature metadata mismatch'):
                release.verify(path, version)
