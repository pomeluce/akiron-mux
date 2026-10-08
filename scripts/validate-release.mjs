import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';

const tag = process.env.RELEASE_TAG;
assert.match(tag ?? '', /^v\d+\.\d+\.\d+$/, 'release tag must be vMAJOR.MINOR.PATCH');

const git = (...args) => execFileSync('git', args, { encoding: 'utf8' }).trim();
const sha = git('rev-parse', '--verify', `refs/tags/${tag}^{commit}`);
const version = tag.slice(1);
for (const path of ['Cargo.toml', 'web/session-ui/src-tauri/Cargo.toml']) {
  const manifest = git('show', `${sha}:${path}`);
  const section = manifest.match(/^\[package\]\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[1];
  assert.equal(section?.match(/^version\s*=\s*"([^"]+)"/m)?.[1], version, `${path} version must match the tag`);
}
for (const path of ['web/session-ui/package.json', 'web/session-ui/src-tauri/tauri.conf.json']) {
  assert.equal(JSON.parse(git('show', `${sha}:${path}`)).version, version, `${path} version must match the tag`);
}
if (process.env.GITHUB_OUTPUT) {
  fs.appendFileSync(process.env.GITHUB_OUTPUT, `source_sha=${sha}\n`);
}
console.log(`Validated ${tag} at ${sha}.`);
