// Generate dependency notices from the exact locked Rust and installed npm packages.
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, existsSync, writeFileSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--format-version', '1'], { cwd: root, maxBuffer: 30 * 1024 * 1024 }));
const lock = JSON.parse(readFileSync(join(root, 'package-lock.json'), 'utf8'));
const packages = metadata.packages.filter(p => p.source).map(p => ({ name: p.name, version: p.version, license: p.license, directory: dirname(p.manifest_path), licenseFile: p.license_file, url: p.repository || `https://crates.io/crates/${p.name}/${p.version}` }));
for (const [path, p] of Object.entries(lock.packages)) {
  // Include build tooling too, so this file conservatively covers bundled native tools.
  if (!path || !existsSync(join(root, path))) continue;
  const info = JSON.parse(readFileSync(join(root, path, 'package.json'), 'utf8'));
  packages.push({ name: info.name, version: p.version, license: p.license || info.license, directory: join(root, path), url: `https://www.npmjs.com/package/${info.name}/v/${p.version}` });
}
let output = '# Third-party notices\n\nGenerated from Cargo.lock and package-lock.json. Includes build and target-specific dependencies; not every listed package is present in every binary. mqx itself is MIT licensed; see LICENSE.\n';
for (const p of packages.sort((a, b) => `${a.name}@${a.version}`.localeCompare(`${b.name}@${b.version}`))) {
  output += `\n## ${p.name} ${p.version}\n\nLicense expression: ${p.license || 'See upstream license'}\n\nSource: ${p.url}\n`;
  const files = readdirSync(p.directory).filter(n => /^(licen[cs]e|copying|notice|copyright)([._-]|$)/i.test(n));
  if (p.licenseFile && !files.includes(p.licenseFile)) files.push(p.licenseFile);
  for (const name of files.sort()) {
    const path = join(p.directory, name);
    if (existsSync(path) && statSync(path).isFile()) output += `\n### ${name}\n\n${readFileSync(path, 'utf8')}\n`;
  }
}
writeFileSync(join(root, 'THIRD_PARTY_NOTICES.md'), output);
console.log(`Generated third-party notices for ${packages.length} locked dependencies.`);
