import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

const [command, tag, assets, destination = 'nix/release-assets.nix'] = process.argv.slice(2);
assert.ok(['generate', 'sync'].includes(command), 'expected generate or sync');
assert.match(tag ?? '', /^v\d+\.\d+\.\d+$/, 'release tag must be vMAJOR.MINOR.PATCH');
assert.ok(assets, 'release assets directory is required');
const version = tag.slice(1);
const cli = `AkironMux-${version}-linux-x86_64-cli.tar.gz`;
const desktop = `AkironMux-${version}-linux-x86_64-desktop.deb`;
const manifest = path.join(assets, `AkironMux-${version}-nix-release-assets.nix`);
const hash = name => createHash('sha256').update(fs.readFileSync(path.join(assets, name))).digest('hex');
const metadata = `{
  version = "${version}";

  systems.x86_64-linux = {
    cli = {
      fileName = "${cli}";
      sha256 = "${hash(cli)}";
    };
    desktop = {
      fileName = "${desktop}";
      sha256 = "${hash(desktop)}";
    };
  };
}
`;

if (command === 'generate') {
  if (fs.existsSync(manifest)) {
    assert.equal(fs.readFileSync(manifest, 'utf8'), metadata, 'same-version release metadata must not change');
  } else {
    fs.writeFileSync(manifest, metadata);
  }
  console.log(`Generated verified Nix metadata for ${tag}.`);
} else {
  assert.equal(fs.readFileSync(manifest, 'utf8'), metadata, 'published asset checksums must match the release metadata');
  const existing = fs.existsSync(destination) ? fs.readFileSync(destination, 'utf8') : null;
  let changed = existing !== metadata;
  if (existing !== null) {
    const currentVersion = existing.match(/\bversion\s*=\s*"(\d+\.\d+\.\d+)";/)?.[1];
    assert.ok(currentVersion, 'existing release metadata must have a version');
    const current = currentVersion.split('.').map(BigInt);
    const proposed = version.split('.').map(BigInt);
    const difference = proposed.map((value, index) => value - current[index]).find(value => value !== 0n) ?? 0n;
    if (difference < 0n) {
      changed = false;
      console.log(`Keeping newer Nix release metadata ${currentVersion}.`);
    } else if (difference === 0n) {
      assert.equal(existing, metadata, 'same-version Nix metadata must not change');
    }
  }
  if (changed) {
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.writeFileSync(destination, metadata);
  }
  if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, `changed=${changed}\n`);
  console.log(changed ? `Updated Nix metadata for ${tag}.` : 'Nix release metadata is unchanged.');
}
