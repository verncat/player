import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, realpathSync, mkdirSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import spdxLicenses from 'spdx-license-list/full.js';

const root = fileURLToPath(new URL('../', import.meta.url));
const licenseName = /^(licen[cs]e|copying|copyright|notice|unlicense)([._-]|$)/i;

export function readLicenseFiles(directory, explicitFile) {
  const files = new Set();
  if (explicitFile && existsSync(resolve(directory, explicitFile))) files.add(resolve(directory, explicitFile));
  function visit(path, depth, licenseDirectory = false) {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = join(path, entry.name);
      if (entry.isFile() && (licenseDirectory || licenseName.test(entry.name))) files.add(child);
      if (entry.isDirectory() && depth > 0 && !['node_modules', 'target', '.git'].includes(entry.name)) {
        visit(child, depth - 1, licenseDirectory || /^(licenses?|licences?)$/i.test(entry.name));
      }
    }
  }
  visit(directory, 3);
  return [...files].sort().map((path) => ({ name: relative(directory, path), text: readFileSync(path, 'utf8').trim() }));
}

export function collectNpm(directory) {
  const seen = new Set();
  const packages = [];
  function visit(manifest, isRoot = false) {
    manifest = realpathSync(manifest);
    if (seen.has(manifest)) return;
    seen.add(manifest);
    const data = JSON.parse(readFileSync(manifest, 'utf8'));
    const base = dirname(manifest);
    if (!isRoot) packages.push({
      ecosystem: 'npm', name: data.name, version: data.version,
      license: typeof data.license === 'string' ? data.license : data.license?.type,
      files: readLicenseFiles(base),
    });
    const required = data.dependencies ?? {};
    const optional = data.optionalDependencies ?? {};
    const require = createRequire(manifest);
    for (const name of Object.keys({ ...required, ...optional }).sort()) {
      const dependency = require.resolve.paths(name)?.map((path) => join(path, name, 'package.json')).find(existsSync);
      if (dependency) visit(dependency);
      else if (!(name in optional)) throw new Error(`Missing installed dependency: ${name} (required by ${data.name})`);
    }
  }
  visit(join(directory, 'package.json'), true);
  return packages;
}

export function renderLicenses(packages, target) {
  const sorted = [...packages].sort((a, b) => `${a.ecosystem}/${a.name}/${a.version}`.localeCompare(`${b.ecosystem}/${b.name}/${b.version}`, 'en'));
  const sections = sorted.map((pkg) => [
    `${pkg.name} ${pkg.version} (${pkg.ecosystem})`,
    `Declared license: ${pkg.license || 'Not specified by this package'}`,
    ...pkg.files.map((file) => `\n${file.name}\n\n${file.text}`),
    ...(pkg.files.length ? [] : standardLicenseTexts(pkg.license)),
  ].join('\n'));
  return `Third-party licenses\n\nGenerated from installed npm runtime packages and Cargo dependencies for ${target}.\nIncludes Rust build dependencies and bundled license and notice files.\nWhere a package supplies only a license identifier, the standard SPDX text is shown and labeled accordingly.\n\n${sections.join('\n\n' + '='.repeat(72) + '\n\n')}\n`;
}

export function standardLicenseTexts(expression) {
  const identifiers = [...new Set((expression ?? '').match(/[A-Za-z0-9][A-Za-z0-9.+-]*/g) ?? [])]
    .filter((id) => !['AND', 'OR', 'WITH'].includes(id));
  if (!identifiers.length) return ['\nThis package does not declare a license or include a license text in its installed source.'];
  return identifiers.map((id) => spdxLicenses[id]
    ? `\nStandard SPDX text: ${id}\n${spdxLicenses[id].url}\n\n${spdxLicenses[id].licenseText}`
    : `\nNo bundled or standard SPDX text is available for ${id}.`);
}

export function collectCargo(target, run = execFileSync) {
  // Tauri's universal target combines two Rust targets; rustc cannot query it directly.
  const targets = target === 'universal-apple-darwin'
    ? ['aarch64-apple-darwin', 'x86_64-apple-darwin'] : [target];
  const packages = new Map();
  for (const platform of targets) {
    const metadata = JSON.parse(run('cargo', [
      'metadata', '--locked', '--format-version', '1', '--filter-platform', platform,
      '--manifest-path', join(root, 'src-tauri/Cargo.toml'),
    ], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }));
    for (const pkg of metadata.packages) {
      if (!metadata.workspace_members.includes(pkg.id)) packages.set(pkg.id, pkg);
    }
  }
  return [...packages.values()];
}

export function generateLicenses() {
  const target = process.env.LICENSES_TARGET || process.env.TAURI_ENV_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET
    || execFileSync('rustc', ['-vV'], { encoding: 'utf8' }).match(/^host: (.+)$/m)?.[1];
  if (!target) throw new Error('Could not determine Rust target for license generation');
  const packages = collectNpm(root);
  for (const pkg of collectCargo(target)) {
    packages.push({ ecosystem: 'Cargo', name: pkg.name, version: pkg.version,
      license: pkg.license, files: readLicenseFiles(dirname(pkg.manifest_path), pkg.license_file) });
  }
  const output = join(root, 'public/licenses/third-party.txt');
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, renderLicenses(packages, target));
  const missing = packages.filter((pkg) => !pkg.files.length && !pkg.license);
  console.log(`Generated licenses for ${packages.length} packages (${target}).`);
  if (missing.length) console.warn(`Packages without a declared license or bundled license text: ${missing.map((pkg) => `${pkg.name}@${pkg.version}`).join(', ')}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) generateLicenses();
