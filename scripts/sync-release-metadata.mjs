import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

const [tag, candidate, destination = 'nix/release-assets.nix'] = process.argv.slice(2);
assert.match(tag ?? '', /^v\d+\.\d+\.\d+$/, 'release tag must be vMAJOR.MINOR.PATCH');
assert.ok(candidate, 'release candidate directory is required');
const metadata = fs.readFileSync(path.join(candidate, 'nix/release-assets.nix'), 'utf8');
const versionOf = text => text.match(/\bversion\s*=\s*"(\d+\.\d+\.\d+)";/)?.[1];
assert.equal(versionOf(metadata), tag.slice(1), 'candidate metadata version must match the tag');

const entries = [...metadata.matchAll(/fileName\s*=\s*"([^"]+)";\s*sha256\s*=\s*"([a-f0-9]{64})";/g)];
const expected = [`AkironMux-${tag.slice(1)}-linux-x86_64-cli.tar.gz`, `AkironMux-${tag.slice(1)}-linux-x86_64-desktop.deb`];
assert.deepEqual(entries.map(entry => entry[1]).sort(), expected.sort(), 'metadata must reference both Nix release assets');
for (const [, name, hash] of entries) {
  const content = fs.readFileSync(path.join(candidate, 'assets', name));
  assert.equal(createHash('sha256').update(content).digest('hex'), hash, `${name} checksum must match the metadata`);
}
assert.equal(fs.readFileSync(path.join(candidate, 'assets', `AkironMux-${tag.slice(1)}-nix-release-assets.nix`), 'utf8'), metadata);

let changed = false;
const existing = fs.existsSync(destination) ? fs.readFileSync(destination, 'utf8') : null;
if (existing !== null) {
  const currentVersion = versionOf(existing);
  assert.ok(currentVersion, 'existing release metadata must have a version');
  const current = currentVersion.split('.').map(BigInt);
  const proposed = tag.slice(1).split('.').map(BigInt);
  const difference = proposed.map((value, index) => value - current[index]).find(value => value !== 0n) ?? 0n;
  if (difference < 0n) {
    console.log(`Keeping newer Nix release metadata ${currentVersion}.`);
    if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, 'changed=false\n');
    process.exit(0);
  }
  if (difference === 0n) assert.equal(existing, metadata, 'same-version Nix metadata must not change');
}
if (existing !== metadata) {
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.writeFileSync(destination, metadata);
  changed = true;
}
if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, `changed=${changed}\n`);
console.log(changed ? `Updated Nix metadata for ${tag}.` : 'Nix release metadata is already current.');
