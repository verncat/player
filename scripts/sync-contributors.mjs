import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, rename, unlink, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = fileURLToPath(new URL('..', import.meta.url));
const manifestPath = 'src/generated/contributors.json';
const avatarDirectory = 'public/generated/contributors';
const avatarTypes = new Map([
  ['image/png', 'png'],
  ['image/jpeg', 'jpg'],
  ['image/webp', 'webp'],
  ['image/gif', 'gif'],
]);

export function parseGitHubRepository(remote) {
  const match = remote.trim().match(/^(?:git@github\.com:|(?:https?|ssh|git):\/\/(?:git@)?github\.com\/)([^/]+)\/([^/]+?)\/?$/i);
  return match ? `${match[1]}/${match[2].replace(/\.git$/, '')}` : null;
}

async function readSnapshot(rootDir) {
  try {
    const snapshot = JSON.parse(await readFile(join(rootDir, manifestPath), 'utf8'));
    if (typeof snapshot.repository === 'string' && Array.isArray(snapshot.contributors)) return snapshot;
  } catch { /* The first build may not have a snapshot yet. */ }
  return null;
}

function resolveRepository(rootDir, snapshot) {
  if (process.env.GITHUB_REPOSITORY) return process.env.GITHUB_REPOSITORY.trim();
  try {
    const remote = execFileSync('git', ['remote', 'get-url', 'origin'], { cwd: rootDir, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
    const repository = parseGitHubRepository(remote);
    if (repository) return repository;
  } catch { /* Source archives can use the saved repository metadata. */ }
  return snapshot?.repository;
}

async function writeManifest(rootDir, snapshot) {
  const output = join(rootDir, manifestPath);
  await mkdir(join(rootDir, 'src/generated'), { recursive: true });
  const serialized = `${JSON.stringify(snapshot, null, 2)}\n`;
  try { if (await readFile(output, 'utf8') === serialized) return; } catch { /* New snapshot. */ }
  await writeFile(`${output}.tmp`, serialized);
  await rename(`${output}.tmp`, output);
}

async function listContributors(repository, fetchImpl, token) {
  const contributors = new Map();
  for (let page = 1; ; page++) {
    const response = await fetchImpl(`https://api.github.com/repos/${repository}/contributors?per_page=100&page=${page}`, {
      headers: {
        Accept: 'application/vnd.github+json',
        'User-Agent': 'player-contributors-build',
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      signal: AbortSignal.timeout(10_000),
    });
    if (response.status === 204) break;
    if (!response.ok) throw new Error(`GitHub returned HTTP ${response.status}`);
    const entries = await response.json();
    if (!Array.isArray(entries)) throw new Error('GitHub returned an invalid contributor list');
    for (const entry of entries) {
      if (typeof entry.login !== 'string' || !Number.isSafeInteger(entry.id)) continue;
      const profile = new URL(entry.html_url);
      if (profile.protocol !== 'https:' || profile.hostname !== 'github.com') continue;
      contributors.set(entry.login.toLowerCase(), entry);
    }
    if (entries.length < 100) break;
  }
  return [...contributors.values()];
}

async function existingAvatar(rootDir, contributor) {
  if (!contributor?.avatarPath?.match(/^generated\/contributors\/\d+-[a-f0-9]{12}\.(png|jpg|webp|gif)$/)) return null;
  try {
    await readFile(join(rootDir, 'public', contributor.avatarPath));
    return contributor.avatarPath;
  } catch { return null; }
}

async function downloadAvatar(rootDir, contributor, fetchImpl) {
  const url = new URL(contributor.avatar_url);
  if (url.protocol !== 'https:' || url.hostname !== 'avatars.githubusercontent.com') throw new Error('Unsupported avatar URL');
  url.searchParams.set('s', '96');
  const response = await fetchImpl(url.href, { signal: AbortSignal.timeout(10_000) });
  if (!response.ok) throw new Error(`Avatar returned HTTP ${response.status}`);
  const extension = avatarTypes.get(response.headers.get('content-type')?.split(';')[0].trim().toLowerCase());
  if (!extension) throw new Error('Unsupported avatar image format');
  const image = Buffer.from(await response.arrayBuffer());
  if (!image.length) throw new Error('Empty avatar image');
  const hash = createHash('sha256').update(image).digest('hex').slice(0, 12);
  const name = `${contributor.id}-${hash}.${extension}`;
  await writeFile(join(rootDir, avatarDirectory, name), image);
  return `generated/contributors/${name}`;
}

export async function syncContributors({ rootDir = projectRoot, repository, fetchImpl = fetch, token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN, logger = console } = {}) {
  const saved = await readSnapshot(rootDir);
  repository ??= resolveRepository(rootDir, saved);
  if (!repository?.match(/^[\w.-]+\/[\w.-]+$/)) throw new Error('Set GITHUB_REPOSITORY to the GitHub owner/repository to build contributor credits');
  const cached = saved?.repository.toLowerCase() === repository.toLowerCase() ? saved : null;
  const previous = new Map((cached?.contributors ?? []).map((entry) => [entry.login.toLowerCase(), entry]));
  let entries;
  try {
    entries = await listContributors(repository, fetchImpl, token);
  } catch {
    const contributors = await Promise.all((cached?.contributors ?? []).map(async (entry) => ({
      login: entry.login, profileUrl: entry.profileUrl, avatarPath: await existingAvatar(rootDir, entry),
    })));
    const snapshot = { repository, contributors };
    await writeManifest(rootDir, snapshot);
    logger.warn(`[contributors] GitHub is unavailable. Using ${contributors.length} saved contributors.`);
    return snapshot;
  }

  await mkdir(join(rootDir, avatarDirectory), { recursive: true });
  const contributors = new Array(entries.length);
  let cursor = 0;
  async function downloadNext() {
    while (cursor < entries.length) {
      const index = cursor++;
      const entry = entries[index];
      let avatarPath;
      try {
        avatarPath = await downloadAvatar(rootDir, entry, fetchImpl);
      } catch {
        avatarPath = await existingAvatar(rootDir, previous.get(entry.login.toLowerCase()));
        logger.warn(`[contributors] Could not refresh the avatar for ${entry.login}. Using a saved image or initials.`);
      }
      contributors[index] = { login: entry.login, profileUrl: entry.html_url, avatarPath };
    }
  }
  await Promise.all(Array.from({ length: Math.min(4, entries.length) }, downloadNext));
  const snapshot = { repository, contributors };
  await writeManifest(rootDir, snapshot);
  const used = new Set(contributors.map((entry) => entry.avatarPath?.split('/').pop()));
  for (const name of await readdir(join(rootDir, avatarDirectory))) {
    if (/^\d+-[a-f0-9]{12}\.(png|jpg|webp|gif)$/.test(name) && !used.has(name)) await unlink(join(rootDir, avatarDirectory, name));
  }
  logger.log(`[contributors] Bundled ${contributors.length} contributors and their avatars from ${repository}.`);
  return snapshot;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await syncContributors();
}
