// Keep the certificate-library audit exceptions limited to the reviewed, unused error type.
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import assert from 'node:assert/strict';
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--locked'], { maxBuffer: 30 * 1024 * 1024 }));
const legacy = metadata.packages.find(p => p.name === 'rustls-webpki' && p.version === '0.102.8');
assert(legacy, 'Remove the old webpki audit exceptions: the reviewed dependency is no longer present.');
const mqtt = metadata.packages.find(p => p.name === 'rumqttc' && p.version === '0.25.1');
assert(mqtt, 'Re-review the audit exceptions after changing rumqttc.');
for (const node of metadata.resolve.nodes) {
  if (node.dependencies.includes(legacy.id)) assert.equal(node.id, mqtt.id, 'Old webpki has a new consumer; review its advisories.');
}
const references = [];
function scan(path) {
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    const file = join(path, entry.name);
    if (entry.isDirectory()) scan(file);
    else if (file.endsWith('.rs')) {
      for (const line of readFileSync(file, 'utf8').split('\n')) {
        if (/\bwebpki\b/.test(line)) references.push(line.trim());
      }
    }
  }
}
scan(join(dirname(mqtt.manifest_path), 'src'));
assert.deepEqual(references, ['WebPki(#[from] webpki::Error),'], 'Old webpki use changed: review the audit exceptions.');
console.log('Verified the old webpki dependency is used only for the reviewed error type.');
