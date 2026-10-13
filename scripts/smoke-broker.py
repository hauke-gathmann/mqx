#!/usr/bin/env python3
"""Test the app's MQTT core against a disposable authenticated TLS Mosquitto broker."""
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
for tool in ('openssl', 'mosquitto', 'mosquitto_passwd'):
    if not shutil.which(tool):
        raise SystemExit(f'{tool} is required for the broker smoke test')

with tempfile.TemporaryDirectory(prefix='mqx-broker-smoke-') as directory:
    directory = Path(directory)
    def run(*args):
        subprocess.run(args, cwd=directory, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    run('openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
        '-keyout', 'ca.key', '-out', 'ca.pem', '-subj', '/CN=mqx disposable test CA',
        '-addext', 'basicConstraints=critical,CA:TRUE')
    run('openssl', 'req', '-newkey', 'rsa:2048', '-nodes', '-keyout', 'server.key',
        '-out', 'server.csr', '-subj', '/CN=localhost')
    (directory / 'server.ext').write_text('subjectAltName=IP:127.0.0.1,DNS:localhost\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\nkeyUsage=digitalSignature,keyEncipherment\n')
    run('openssl', 'x509', '-req', '-in', 'server.csr', '-CA', 'ca.pem', '-CAkey', 'ca.key',
        '-CAcreateserial', '-out', 'server.pem', '-days', '1', '-extfile', 'server.ext')
    run('mosquitto_passwd', '-b', '-c', 'passwords', 'mqx-test', 'disposable-test-password')
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    config = directory / 'mosquitto.conf'
    config.write_text(f'''listener {port} 127.0.0.1
allow_anonymous false
password_file {directory / 'passwords'}
cafile {directory / 'ca.pem'}
certfile {directory / 'server.pem'}
keyfile {directory / 'server.key'}
persistence false
''')
    with (directory / 'broker.log').open('w+') as log:
        broker = subprocess.Popen(['mosquitto', '-c', str(config)], stdout=log, stderr=log)
        try:
            for attempt in range(100):
                if broker.poll() is not None:
                    log.seek(0)
                    raise RuntimeError(log.read())
                try:
                    with socket.create_connection(('127.0.0.1', port), timeout=0.1):
                        break
                except OSError:
                    time.sleep(0.05)
            else:
                raise RuntimeError('Disposable broker did not start')
            subprocess.run(['cargo', 'test', '--locked', '-p', 'mqx-core', '--test', 'broker_release', '--', '--ignored'],
                           cwd=ROOT, env=dict(os.environ, MQX_TEST_BROKER_PORT=str(port), MQX_TEST_CA=str(directory / 'ca.pem')),
                           check=True, timeout=180)
        finally:
            broker.terminate()
            try:
                broker.wait(timeout=5)
            except subprocess.TimeoutExpired:
                broker.kill()
                broker.wait()
