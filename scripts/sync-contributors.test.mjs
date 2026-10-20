import assert from 'node:assert/strict';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { parseGitHubRepository, syncContributors } from './sync-contributors.mjs';

const image = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=', 'base64');
const repository = 'example/player';
const logger = { log() {}, warn() {} };
const contributor = (id) => ({ id, login: `user-${id}`, html_url: `https://github.com/user-${id}`, avatar_url: `https://avatars.githubusercontent.com/u/${id}` });
const json = (value) => new Response(JSON.stringify(value), { headers: { 'content-type': 'application/json' } });
const avatar = (bytes = image) => new Response(bytes, { headers: { 'content-type': 'image/png' } });
async function fixture(t) {
  const rootDir = await mkdtemp(join(tmpdir(), 'player-contributors-'));
  t.after(() => rm(rootDir, { recursive: true, force: true }));
  return rootDir;
}
const oneContributor = async (url) => url.startsWith('https://api.github.com/') ? json([contributor(1)]) : avatar();

test('resolves GitHub repositories from HTTPS and SSH remotes', () => {
  for (const remote of ['https://github.com/example/player.git', 'git@github.com:example/player.git', 'ssh://git@github.com/example/player.git', 'https://github.com/example/player/']) {
    assert.equal(parseGitHubRepository(remote), repository);
  }
  assert.equal(parseGitHubRepository('https://gitlab.com/example/player.git'), null);
});

test('includes every contributor page and bundles avatars without sending the API token to the image host', async (t) => {
  const rootDir = await fixture(t);
  const apiPages = [];
  const result = await syncContributors({ rootDir, repository, logger, token: 'test-build-token', fetchImpl: async (url, options) => {
    const request = new URL(url);
    if (request.hostname === 'api.github.com') {
      assert.equal(options.headers.Authorization, 'Bearer test-build-token');
      const page = Number(request.searchParams.get('page'));
      apiPages.push(page);
      return json(page === 1 ? Array.from({ length: 100 }, (_, index) => contributor(index + 1)) : [contributor(101)]);
    }
    assert.equal(request.hostname, 'avatars.githubusercontent.com');
    assert.equal(request.searchParams.get('s'), '96');
    assert.equal(options.headers?.Authorization, undefined);
    return avatar();
  } });
  assert.deepEqual(apiPages, [1, 2]);
  assert.equal(result.contributors.length, 101);
  assert.equal((await readdir(join(rootDir, 'public/generated/contributors'))).length, 101);
  for (const entry of result.contributors) {
    assert.deepEqual(await readFile(join(rootDir, 'public', entry.avatarPath)), image);
    assert.ok(entry.profileUrl.startsWith('https://github.com/'));
  }
});

test('keeps the bundled contributor list and images when GitHub is unavailable or rate limited', async (t) => {
  const rootDir = await fixture(t);
  const saved = await syncContributors({ rootDir, repository, logger, fetchImpl: oneContributor });
  for (const fetchImpl of [async () => { throw new Error('Offline'); }, async () => new Response('', { status: 403 })]) {
    const result = await syncContributors({ rootDir, repository, logger, fetchImpl });
    assert.deepEqual(result, saved);
    assert.deepEqual(await readFile(join(rootDir, 'public', result.contributors[0].avatarPath)), image);
    assert.deepEqual(JSON.parse(await readFile(join(rootDir, 'src/generated/contributors.json'), 'utf8')), saved);
  }
});

test('uses a cached avatar for an existing contributor and initials for a new contributor if image downloads fail', async (t) => {
  const rootDir = await fixture(t);
  const saved = await syncContributors({ rootDir, repository, logger, fetchImpl: oneContributor });
  const result = await syncContributors({ rootDir, repository, logger, fetchImpl: async (url) => {
    if (url.startsWith('https://api.github.com/')) return json([contributor(1), contributor(2)]);
    return new Response('', { status: 503 });
  } });
  assert.equal(result.contributors[0].avatarPath, saved.contributors[0].avatarPath);
  assert.equal(result.contributors[1].avatarPath, null);
  assert.equal(result.contributors[1].profileUrl, 'https://github.com/user-2');
});

test('replaces changed avatars and only removes old generated avatar files', async (t) => {
  const rootDir = await fixture(t);
  const saved = await syncContributors({ rootDir, repository, logger, fetchImpl: oneContributor });
  await writeFile(join(rootDir, 'public/generated/contributors/keep.txt'), 'Keep this file');
  const result = await syncContributors({ rootDir, repository, logger, fetchImpl: async (url) => url.startsWith('https://api.github.com/') ? json([contributor(1)]) : avatar(Buffer.concat([image, Buffer.from('updated')])) });
  assert.notEqual(result.contributors[0].avatarPath, saved.contributors[0].avatarPath);
  await assert.rejects(readFile(join(rootDir, 'public', saved.contributors[0].avatarPath)), { code: 'ENOENT' });
  assert.equal(await readFile(join(rootDir, 'public/generated/contributors/keep.txt'), 'utf8'), 'Keep this file');
});

test('allows a first offline build without borrowing credits from a different repository', async (t) => {
  const rootDir = await fixture(t);
  const offline = async () => { throw new Error('Offline'); };
  assert.deepEqual(await syncContributors({ rootDir, repository, logger, fetchImpl: offline }), { repository, contributors: [] });
  await syncContributors({ rootDir, repository, logger, fetchImpl: oneContributor });
  assert.deepEqual(await syncContributors({ rootDir, repository: 'other/player', logger, fetchImpl: offline }), { repository: 'other/player', contributors: [] });
});
