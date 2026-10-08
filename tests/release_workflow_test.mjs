import assert from 'node:assert/strict';
import fs from 'node:fs';

const workflow = fs.readFileSync(new URL('../.github/workflows/release.yml', import.meta.url), 'utf8');
const jobs = Object.fromEntries([...workflow.matchAll(/^  ([\w-]+):\n([\s\S]*?)(?=^  [\w-]+:\n|$(?![\s\S]))/gm)].map(match => [match[1], match[2]]));
const buildJobs = ['linux', 'macos', 'windows', 'gui-linux', 'gui-macos', 'gui-windows'];

assert.match(workflow, /^on:\n  push:\n    tags:\n      - ['"]v\*['"]/m, 'pushing a version tag must trigger Release');
assert.doesNotMatch(workflow, /candidate_run_id|inputs\.phase|release-candidate/, 'release must not require phase or candidate orchestration');
assert.ok(!jobs.prepare, 'build and publish must be one continuous run');
assert.ok(!fs.existsSync(new URL('../.github/workflows/release-ssh.yml', import.meta.url)), 'remove the obsolete SSH trigger workflow');
assert.match(workflow, /group: release-/, 'same-tag runs must be serialized');
assert.match(jobs.validate, /scripts\/validate-release\.mjs/, 'validate the tag before building');

for (const job of buildJobs) {
  assert.doesNotMatch(jobs[job], /^    if:/m, `${job} must build on both tag pushes and manual dispatch without phase inputs`);
  assert.match(jobs[job], /needs: validate/);
  assert.match(jobs[job], /ref: \$\{\{ needs\.validate\.outputs\.source_sha \}\}/, 'build the requested tag, not the dispatch branch');
}

const dependencies = jobs.publish.match(/^    needs: \[([^\]]+)\]/m)?.[1].split(',').map(name => name.trim());
assert.deepEqual(dependencies, ['validate', ...buildJobs], 'publish must wait for every successful build and source validation');
assert.doesNotMatch(jobs.publish, /^    if:/m, 'retain the default success gate: failed or skipped builds must block publication');
assert.match(jobs.publish, /actions\/download-artifact@/);
assert.match(jobs.publish, /scripts\/release-metadata\.mjs generate/);
assert.doesNotMatch(jobs.publish, /run-id:|actions\/upload-artifact@/, 'publish this run directly without an intermediate candidate upload');
assert.doesNotMatch(jobs.publish, /cmp nix\/release-assets\.nix/, 'a new tag cannot already contain hashes of not-yet-built assets');
assert.match(jobs['sync-nix'], /needs: publish/);
assert.match(jobs['sync-nix'], /gh release download/, 'sync checksums against the actual published files');
assert.match(jobs['sync-nix'], /github\.event\.repository\.default_branch/);
assert.match(jobs['sync-nix'], /add-paths: nix\/release-assets\.nix/, 'the metadata PR must not include unrelated files');
assert.match(jobs['sync-nix'], /scripts\/release-metadata\.mjs sync/);
assert.match(jobs.homebrew, /needs: publish/, 'Homebrew failure must not gate publication or Nix synchronization');

console.log('Release tag trigger and job routing regression tests passed.');
