import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'akmux-release-metadata-'));
const script = fileURLToPath(new URL('../scripts/release-metadata.mjs', import.meta.url));
const assets = path.join(root, 'assets');
const manifest = path.join(assets, 'AkironMux-2.0.0-nix-release-assets.nix');
const destination = path.join(root, 'nix/release-assets.nix');
const output = path.join(root, 'output');
const run = (command, tag = 'v2.0.0', target = destination) => spawnSync(process.execPath, [script, command, tag, assets, target], {
  encoding: 'utf8', env: { ...process.env, GITHUB_OUTPUT: output },
});
const succeeds = result => assert.equal(result.status, 0, result.stderr);

try {
  fs.mkdirSync(assets);
  const cli = path.join(assets, 'AkironMux-2.0.0-linux-x86_64-cli.tar.gz');
  const desktop = path.join(assets, 'AkironMux-2.0.0-linux-x86_64-desktop.deb');
  fs.writeFileSync(cli, 'cli asset\n');
  assert.notEqual(run('generate').status, 0, 'both Nix assets are required');
  assert.ok(!fs.existsSync(manifest), 'failure must not leave a partial manifest');
  fs.writeFileSync(desktop, 'desktop asset\n');
  succeeds(run('generate'));
  const metadata = fs.readFileSync(manifest, 'utf8');
  for (const file of [cli, desktop]) {
    const digest = createHash('sha256').update(fs.readFileSync(file)).digest('hex');
    assert.ok(metadata.includes(`sha256 = "${digest}";`), 'use the checksum of the actual file');
  }
  succeeds(run('generate'));
  succeeds(run('sync'));
  assert.equal(fs.readFileSync(destination, 'utf8'), metadata);
  assert.ok(fs.readFileSync(output, 'utf8').endsWith('changed=true\n'));
  succeeds(run('sync'));
  assert.ok(fs.readFileSync(output, 'utf8').endsWith('changed=false\n'));
  fs.writeFileSync(destination, metadata.replace('version = "2.0.0"', 'version = "3.0.0"'));
  succeeds(run('sync'));
  assert.ok(fs.readFileSync(destination, 'utf8').includes('version = "3.0.0"'), 'do not downgrade newer metadata');
  fs.writeFileSync(destination, metadata + '# conflicting same-version metadata\n');
  assert.notEqual(run('sync').status, 0, 'reject same-version metadata conflicts');
  fs.writeFileSync(destination, metadata);
  fs.writeFileSync(manifest, metadata + '# altered published manifest\n');
  assert.notEqual(run('sync').status, 0, 'verify the published manifest, not only filenames');
  assert.equal(fs.readFileSync(destination, 'utf8'), metadata);
  fs.writeFileSync(manifest, metadata);
  fs.appendFileSync(cli, 'tampered asset\n');
  assert.notEqual(run('sync').status, 0, 'reject changed published binary checksums');
  assert.notEqual(run('generate').status, 0, 'do not overwrite existing same-version metadata');
  assert.equal(fs.readFileSync(manifest, 'utf8'), metadata);
  assert.equal(fs.readFileSync(destination, 'utf8'), metadata);
  for (const tag of ['v1.0.0', 'main', '../v2.0.0']) assert.notEqual(run('sync', tag).status, 0);
  assert.notEqual(run('unknown').status, 0);
  console.log('Release metadata checksum and synchronization regression tests passed.');
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}
