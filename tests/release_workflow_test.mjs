import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const workflow = fs.readFileSync(new URL('../.github/workflows/release.yml', import.meta.url), 'utf8');
const jobs = Object.fromEntries([...workflow.matchAll(/^  ([\w-]+):\n([\s\S]*?)(?=^  [\w-]+:\n|$(?![\s\S]))/gm)].map(match => [match[1], match[2]]));
const buildJobs = ['linux', 'macos', 'windows', 'gui-linux', 'gui-macos', 'gui-windows'];

function runs(job, event, phase, prepare = 'success', validated = 'success') {
  const expression = jobs[job]?.match(/^    if: (.*)$/m)?.[1];
  assert.ok(expression, `${job} must have an explicit condition`);
  return vm.runInNewContext(expression, {
    github: { event_name: event },
    inputs: { phase },
    needs: { validate: { result: validated }, prepare: { result: prepare }, publish: { result: 'success' } },
    always: () => true,
  });
}

assert.match(workflow, /^on:\n  push:\n    tags:\n      - ['"]v\*['"]/m, 'pushing a version tag must trigger Release');
assert.match(workflow, /default: release/, 'manual dispatch must release by default');
assert.match(workflow, /group: release-/, 'same-tag runs must be serialized');
assert.match(jobs.validate, /scripts\/validate-release\.mjs/, 'validate the tag before building');

for (const job of buildJobs) {
  assert.equal(runs(job, 'push', ''), true, `${job} must build on tag pushes without inputs`);
  assert.equal(runs(job, 'workflow_dispatch', 'release'), true);
  assert.equal(runs(job, 'workflow_dispatch', 'prepare'), true);
  assert.equal(runs(job, 'workflow_dispatch', 'publish'), false);
  assert.match(jobs[job], /needs: validate/);
  assert.match(jobs[job], /ref: \$\{\{ needs\.validate\.outputs\.source_sha \}\}/, 'build the requested tag, not the dispatch branch');
}

assert.equal(runs('publish', 'push', ''), true);
assert.equal(runs('publish', 'workflow_dispatch', 'release'), true);
assert.equal(runs('publish', 'workflow_dispatch', 'prepare'), false);
assert.equal(runs('publish', 'workflow_dispatch', 'publish', 'skipped'), true, 'explicit publish must survive skipped build jobs');
assert.equal(runs('publish', 'push', '', 'failure'), false, 'failed builds must never publish');
assert.equal(runs('publish', 'push', '', 'skipped'), false);
assert.equal(runs('publish', 'push', '', 'success', 'failure'), false);
assert.match(jobs.publish, /needs: \[validate, prepare\]/);
assert.match(jobs.publish, /source-commit/, 'bind prepared artifacts to the tag commit');
assert.match(jobs.publish, /inputs\.candidate_run_id \|\| github\.run_id/, 'use this run unless publishing a stored candidate');
assert.doesNotMatch(jobs.publish, /cmp nix\/release-assets\.nix/, 'a new tag cannot already contain hashes of not-yet-built assets');
assert.match(jobs['sync-nix'], /needs: \[validate, prepare, publish\]/);
assert.match(jobs['sync-nix'], /github\.event\.repository\.default_branch/);
assert.match(jobs['sync-nix'], /add-paths: nix\/release-assets\.nix/, 'the metadata PR must not include unrelated files');
assert.match(jobs['sync-nix'], /scripts\/sync-release-metadata\.mjs/);

console.log('Release tag trigger and job routing regression tests passed.');
