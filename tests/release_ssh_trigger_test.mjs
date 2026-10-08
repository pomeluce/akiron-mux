import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const workflowPath = new URL('../.github/workflows/release-ssh.yml', import.meta.url);
assert.ok(fs.existsSync(workflowPath), 'SSH push needs a release-trigger branch workflow');
const workflow = fs.readFileSync(workflowPath, 'utf8');
assert.match(workflow, /^on:\n  push:\n    branches:\n      - ['"]release-trigger\/v\*['"]/m);
assert.match(workflow, /permissions:\n  actions: write\n  contents: read/);
const script = workflow.match(/          script: \|\n([\s\S]*)/)?.[1].replace(/^            /gm, '');
assert.ok(script, 'the trigger must dispatch the canonical Release workflow');

async function trigger(ref, tagExists = true, defaultBranch = 'main') {
  const calls = [];
  const promise = vm.runInNewContext(`(async () => {${script}\n})()`, {
    context: { ref, repo: { owner: 'fixture', repo: 'akmux' }, payload: { repository: { default_branch: defaultBranch } } },
    github: { rest: {
      git: { getRef: async args => {
        calls.push(['getRef', args]);
        if (!tagExists) throw new Error('tag not found');
      } },
      actions: { createWorkflowDispatch: async args => { calls.push(['dispatch', args]); } },
    } },
    core: { notice: () => {} },
  });
  return { calls, promise };
}

const successful = await trigger('refs/heads/release-trigger/v1.15.5');
await successful.promise;
assert.deepEqual(JSON.parse(JSON.stringify(successful.calls)), [
  ['getRef', { owner: 'fixture', repo: 'akmux', ref: 'tags/v1.15.5' }],
  ['dispatch', { owner: 'fixture', repo: 'akmux', workflow_id: 'release.yml', ref: 'main', inputs: { tag: 'v1.15.5', phase: 'release' } }],
]);

const otherDefault = await trigger('refs/heads/release-trigger/v2.0.0', true, 'trunk');
await otherDefault.promise;
assert.equal(otherDefault.calls[1][1].ref, 'trunk');
for (const ref of ['refs/heads/main', 'refs/tags/v1.15.5', 'refs/heads/release-trigger/v1.15.5/extra', 'refs/heads/release-trigger/v1.15.5;echo bad']) {
  const invalid = await trigger(ref);
  await assert.rejects(invalid.promise);
  assert.equal(invalid.calls.length, 0, 'invalid branches must never request a release');
}
const missing = await trigger('refs/heads/release-trigger/v1.15.5', false);
await assert.rejects(missing.promise, /tag not found/);
assert.equal(missing.calls.length, 1, 'a missing tag must not be created or dispatched');
const missingDefault = await trigger('refs/heads/release-trigger/v1.15.5', true, '');
await assert.rejects(missingDefault.promise);
assert.ok(missingDefault.calls.every(([kind]) => kind !== 'dispatch'));

console.log('SSH release trigger regression tests passed.');
