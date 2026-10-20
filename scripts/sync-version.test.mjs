import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test('version sync updates the app lock entry, preserves dependencies, and is idempotent', () => {
  const root = mkdtempSync(join(tmpdir(), 'player-version-'));
  try {
    mkdirSync(join(root, 'scripts'));
    mkdirSync(join(root, 'src-tauri'));
    const script = join(root, 'scripts/sync-version.mjs');
    copyFileSync(new URL('./sync-version.mjs', import.meta.url), script);
    writeFileSync(join(root, 'package.json'), JSON.stringify({ version: '0.3.0' }));
    writeFileSync(join(root, 'src-tauri/Cargo.toml'), '[package]\nname = "player"\nversion = "0.2.11"\n');
    writeFileSync(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify({ version: '0.2.11' }));
    const lock = 'version = 4\n\n[[package]]\nname = "dependency"\nversion = "0.2.11"\n\n[[package]]\nname = "player"\nversion = "0.2.11"\ndependencies = ["dependency"]\n';
    const lockPath = join(root, 'src-tauri/Cargo.lock');
    writeFileSync(lockPath, lock);
    execFileSync(process.execPath, [script]);
    assert.equal(readFileSync(lockPath, 'utf8'), lock.replace('name = "player"\nversion = "0.2.11"', 'name = "player"\nversion = "0.3.0"'));
    assert.match(readFileSync(join(root, 'src-tauri/Cargo.toml'), 'utf8'), /version = "0.3.0"/);
    assert.equal(JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8')).version, '0.3.0');
    assert.match(execFileSync(process.execPath, [script], { encoding: 'utf8' }), /already synced/);
    // Also repair a stale lock when the manifests already have the current version.
    writeFileSync(lockPath, lock);
    execFileSync(process.execPath, [script]);
    assert.match(readFileSync(lockPath, 'utf8'), /name = "player"\nversion = "0.3.0"/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
