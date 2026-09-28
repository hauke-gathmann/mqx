import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
MOCK = r'''#!/usr/bin/env python3
import json, os, pathlib, shutil, subprocess, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
fixture = pathlib.Path(os.environ['MQX_TEST_FIXTURE'])
if name == 'uname':
    print(os.environ['MQX_TEST_OS'] if args == ['-s'] else os.environ['MQX_TEST_ARCH'])
elif name == 'curl':
    url = next(a for a in args if a.startswith('https://'))
    with (fixture / 'requests').open('a') as f: f.write(url + '\n')
    source = fixture / ('release.json' if '/api.github.com/' in url else url.rsplit('/', 1)[1])
    if not source.exists(): sys.exit(22)
    shutil.copyfile(source, args[args.index('-o') + 1])
elif name == 'hdiutil':
    if args[0] == 'attach':
        target = pathlib.Path(args[args.index('-mountpoint') + 1]) / 'mqx.app/Contents/MacOS'
        target.mkdir(parents=True)
        (target / 'mqx').write_text('new app')
elif name == 'ditto':
    if os.environ.get('MQX_TEST_FAIL_COPY'): sys.exit(1)
    shutil.copytree(args[0], args[1])
elif name == 'codesign':
    if os.environ.get('MQX_TEST_BAD_SIGNATURE'): sys.exit(1)
elif name == 'pgrep':
    sys.exit(0 if os.environ.get('MQX_TEST_RUNNING') else 1)
elif name == 'mv':
    if os.environ.get('MQX_TEST_FAIL_REPLACE') and args[0].endswith('/mqx.app') and '.mqx-stage.' in args[0]: sys.exit(1)
    sys.exit(subprocess.call(['/bin/mv'] + args))
'''


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='mqx installer ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fixture = self.root / 'fixtures'
        self.fixture.mkdir()
        tools = self.root / 'tools'
        tools.mkdir()
        dispatcher = tools / 'mock'
        dispatcher.write_text(MOCK)
        dispatcher.chmod(0o755)
        for name in ('uname', 'curl', 'hdiutil', 'ditto', 'codesign', 'pgrep', 'mv'):
            (tools / name).symlink_to(dispatcher)
        self.env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'],
                        MQX_TEST_FIXTURE=str(self.fixture), MQX_TEST_OS='Linux', MQX_TEST_ARCH='x86_64',
                        MQX_BIN=str(self.root / 'bin'), XDG_DATA_HOME=str(self.root / 'data'),
                        MQX_APP_DIR=str(self.root / 'Applications'))
        names = ['mqx_0.2.0_amd64.AppImage', 'mqx_0.2.0_aarch64.dmg', 'mqx_0.2.0_x64.dmg', 'mqx.png']
        for name in names:
            (self.fixture / name).write_bytes(b'artifact-' + name.encode())
        (self.fixture / 'release.json').write_text(json.dumps({'assets': [
            {'browser_download_url': 'https://github.com/hauke-gathmann/mqx/releases/download/v0.2.0/' + n}
            for n in names]}, indent=2))
        (self.fixture / 'SHA256SUMS').write_text(''.join(
            hashlib.sha256((self.fixture / n).read_bytes()).hexdigest() + '  ' + n + '\n' for n in names))

    def run_installer(self, *args, success=True):
        result = subprocess.run(['sh', str(ROOT / 'install.sh'), *args], env=self.env, capture_output=True, text=True, errors="replace")
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def mac(self, arch='arm64'):
        self.env.update(MQX_TEST_OS='Darwin', MQX_TEST_ARCH=arch)
        old = self.root / 'Applications/mqx.app/old'
        old.parent.mkdir(parents=True)
        old.write_text('working app')
        return old

    def test_linux_install_launcher_icon_and_uninstall_preserves_data(self):
        profile = self.root / 'data/mqx/connections.json'
        profile.parent.mkdir(parents=True)
        profile.write_text('saved profile')
        self.run_installer()
        self.assertTrue((self.root / 'bin/mqx').is_file())
        self.assertIn(str(self.root / 'bin/mqx'), (self.root / 'data/applications/dev.mqx.app.desktop').read_text())
        self.assertTrue((self.root / 'data/icons/hicolor/128x128/apps/dev.mqx.app.png').is_file())
        self.run_installer('--uninstall')
        self.assertEqual(profile.read_text(), 'saved profile')
        self.assertFalse((self.root / 'bin/mqx').exists())

    def test_pinned_version_uses_tag_endpoint(self):
        self.run_installer('--version', 'v0.2.0')
        self.assertIn('/releases/tags/v0.2.0', (self.fixture / 'requests').read_text())

    def test_checksum_failure_preserves_existing_install(self):
        dest = self.root / 'data/mqx/app/mqx.AppImage'
        dest.parent.mkdir(parents=True)
        dest.write_text('working app')
        (self.fixture / 'mqx_0.2.0_amd64.AppImage').write_text('corrupt')
        self.run_installer(success=False)
        self.assertEqual(dest.read_text(), 'working app')

    def test_missing_checksum_manifest_fails(self):
        (self.fixture / 'SHA256SUMS').unlink()
        self.run_installer(success=False)
        self.assertFalse((self.root / 'bin/mqx').exists())

    def test_missing_download_fails(self):
        (self.fixture / 'mqx_0.2.0_amd64.AppImage').unlink()
        self.run_installer(success=False)

    def test_no_platform_asset_fails(self):
        (self.fixture / 'release.json').write_text('{}')
        self.run_installer(success=False)

    def test_unsupported_linux_arm_fails_before_network(self):
        self.env['MQX_TEST_ARCH'] = 'aarch64'
        self.run_installer(success=False)
        self.assertFalse((self.fixture / 'requests').exists())

    def test_does_not_replace_unrelated_command(self):
        command = self.root / 'bin/mqx'
        command.parent.mkdir()
        command.write_text('unrelated')
        self.run_installer(success=False)
        self.assertEqual(command.read_text(), 'unrelated')

    def test_mac_architectures_and_replacement(self):
        for arch in ['arm64', 'x86_64']:
            with self.subTest(arch=arch):
                app = self.root / 'Applications/mqx.app'
                if app.exists(): shutil.rmtree(app)
                old = self.mac(arch)
                self.run_installer()
                self.assertFalse(old.exists())
                self.assertEqual((app / 'Contents/MacOS/mqx').read_text(), 'new app')
                self.assertFalse(list(app.parent.glob('.mqx-*')))

    def test_mac_copy_failure_preserves_old_app(self):
        old = self.mac()
        self.env['MQX_TEST_FAIL_COPY'] = '1'
        self.run_installer(success=False)
        self.assertEqual(old.read_text(), 'working app')

    def test_mac_replacement_failure_restores_old_app(self):
        old = self.mac()
        self.env['MQX_TEST_FAIL_REPLACE'] = '1'
        self.run_installer(success=False)
        self.assertEqual(old.read_text(), 'working app')

    def test_mac_signature_failure_preserves_old_app(self):
        old = self.mac()
        self.env['MQX_TEST_BAD_SIGNATURE'] = '1'
        self.run_installer(success=False)
        self.assertEqual(old.read_text(), 'working app')

    def test_running_mac_app_is_not_replaced(self):
        old = self.mac()
        self.env['MQX_TEST_RUNNING'] = '1'
        self.run_installer(success=False)
        self.assertEqual(old.read_text(), 'working app')

    def test_unsafe_version_rejected(self):
        self.run_installer('--version', 'v0.2.0/../main', success=False)
        self.assertFalse((self.fixture / 'requests').exists())


if __name__ == '__main__':
    unittest.main()
