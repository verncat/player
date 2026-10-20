import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { collectCargo, collectNpm, readLicenseFiles, renderLicenses, standardLicenseTexts } from './generate-licenses.mjs';

test('universal macOS licenses include both architectures without duplicate packages or workspace members', () => {
  const queried = [];
  const packages = collectCargo('universal-apple-darwin', (command, args) => {
    assert.equal(command, 'cargo');
    assert.ok(args.includes('--locked'));
    const platform = args[args.indexOf('--filter-platform') + 1];
    queried.push(platform);
    return JSON.stringify({
      workspace_members: ['app'],
      packages: [{ id: 'app' }, { id: 'common' }, { id: platform }],
    });
  });
  assert.deepEqual(queried, ['aarch64-apple-darwin', 'x86_64-apple-darwin']);
  assert.deepEqual(packages.map((pkg) => pkg.id), ['common', ...queried]);
});

test('ordinary Rust targets pass through unchanged and metadata failures propagate', () => {
  collectCargo('x86_64-pc-windows-msvc', (command, args) => {
    assert.equal(args[args.indexOf('--filter-platform') + 1], 'x86_64-pc-windows-msvc');
    return JSON.stringify({ workspace_members: [], packages: [] });
  });
  assert.throws(() => collectCargo('universal-apple-darwin', () => {
    throw new Error('metadata failed');
  }), /metadata failed/);
});

function fixture(t) {
  const path = mkdtempSync(join(tmpdir(), 'player-licenses-'));
  t.after(() => rmSync(path, { recursive: true, force: true }));
  return path;
}

test('collects transitive installed dependencies once and skips missing optional packages', (t) => {
  const root = fixture(t);
  writeFileSync(join(root, 'package.json'), JSON.stringify({ name: 'app', dependencies: { one: '1', two: '1' } }));
  for (const name of ['one', 'two']) {
    const dir = join(root, 'node_modules', name);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, 'package.json'), JSON.stringify({ name, version: '1.0.0', license: 'MIT', dependencies: name === 'one' ? { two: '1' } : {}, optionalDependencies: { unavailable: '1' } }));
    writeFileSync(join(dir, 'LICENSE'), `License for ${name}`);
  }
  const packages = collectNpm(root);
  assert.deepEqual(packages.map((pkg) => pkg.name), ['one', 'two']);
  assert.equal(packages[1].files[0].text, 'License for two');
  rmSync(join(root, 'node_modules/two'), { recursive: true });
  assert.throws(() => collectNpm(root), /Missing installed dependency: two/);
});

test('includes custom license files and nested notices without reading dependency directories', (t) => {
  const root = fixture(t);
  mkdirSync(join(root, 'licenses'), { recursive: true });
  mkdirSync(join(root, 'node_modules/other'), { recursive: true });
  writeFileSync(join(root, 'terms.txt'), 'Custom terms');
  writeFileSync(join(root, 'licenses/MIT.txt'), 'MIT terms');
  writeFileSync(join(root, 'NOTICE'), 'Author notice');
  writeFileSync(join(root, 'node_modules/other/LICENSE'), 'Not part of this package');
  assert.deepEqual(readLicenseFiles(root, 'terms.txt').map((file) => file.name), ['NOTICE', 'licenses/MIT.txt', 'terms.txt']);
});

test('uses complete SPDX texts for declared licenses without inventing missing metadata', () => {
  const texts = standardLicenseTexts('MIT OR Apache-2.0');
  assert.equal(texts.length, 2);
  assert.match(texts[0], /Permission is hereby granted/);
  assert.match(texts[1], /END OF TERMS AND CONDITIONS/);
  assert.match(standardLicenseTexts(null)[0], /does not declare a license/);
});

test('output is deterministic, preserves notices, and contains no machine paths', () => {
  const packages = [
    { ecosystem: 'npm', name: 'z', version: '1', license: 'MIT', files: [{ name: 'NOTICE', text: 'Original copyright' }] },
    { ecosystem: 'Cargo', name: 'a', version: '2', license: 'MPL-2.0', files: [] },
  ];
  const output = renderLicenses(packages, 'test-target');
  assert.equal(output, renderLicenses([...packages].reverse(), 'test-target'));
  assert.match(output, /Original copyright/);
  assert.match(output, /Mozilla Public License/);
  assert.doesNotMatch(output, /\/Users\/|\/node_modules\//);
});
