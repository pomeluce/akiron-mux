import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'akmux-release-validation-'));
const script = fileURLToPath(new URL('../scripts/validate-release.mjs', import.meta.url));
const git = (...args) => execFileSync('git', ['-c', 'user.name=Release Test', '-c', 'user.email=release@example.invalid', ...args], { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const write = (name, content) => {
  fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
  fs.writeFileSync(path.join(root, name), content);
};
function validate(tag, phase = 'release', runId = '') {
  return spawnSync(process.execPath, [script], {
    cwd: root,
    env: { ...process.env, RELEASE_TAG: tag, RELEASE_PHASE: phase, CANDIDATE_RUN_ID: runId, GITHUB_OUTPUT: path.join(root, 'output') },
    encoding: 'utf8',
  });
}

try {
  git('init', '--initial-branch=main');
  for (const name of ['Cargo.toml', 'web/session-ui/src-tauri/Cargo.toml']) write(name, '[package]\nname = "fixture"\nversion = "2.0.0"\n');
  for (const name of ['web/session-ui/package.json', 'web/session-ui/src-tauri/tauri.conf.json']) write(name, JSON.stringify({ version: '2.0.0' }));
  git('add', '.');
  git('commit', '-m', 'test: prepare release fixture', '-m', 'Exercise release validation against an immutable tag.');
  const sha = git('rev-parse', 'HEAD');
  git('tag', '-a', 'v2.0.0', '-m', 'Release fixture');

  // A manual run on a newer branch must still validate the requested tag.
  write('Cargo.toml', '[package]\nversion = "3.0.0"\n');
  git('add', 'Cargo.toml');
  git('commit', '-m', 'test: advance release fixture', '-m', 'Ensure dispatch validates tag versions instead of branch versions.');
  git('tag', 'v3.0.0');

  for (const phase of ['release', 'prepare', 'publish']) {
    const result = validate('v2.0.0', phase, phase === 'publish' ? '123' : '');
    assert.equal(result.status, 0, result.stderr);
  }
  assert.ok(fs.readFileSync(path.join(root, 'output'), 'utf8').includes(`source_sha=${sha}\n`));
  for (const [tag, phase, runId] of [
    ['v2.0.0', 'publish', ''],
    ['v2.0.0', 'publish', '../123'],
    ['v2.0.0', 'release', '123'],
    ['v2.0.0', 'prepare', '123'],
    ['v2.0.0', 'unknown', ''],
    ['v2.0.1', 'release', ''],
    ['v3.0.0', 'release', ''],
    ['main', 'release', ''],
    ['v2.0.0;echo invalid', 'release', ''],
  ]) {
    assert.notEqual(validate(tag, phase, runId).status, 0, `${tag}/${phase}/${runId} must be rejected`);
  }
  console.log('Release tag source and version validation regression tests passed.');
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}
