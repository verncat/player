import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(scriptDir, '..');

const packageJsonPath = path.join(rootDir, 'package.json');
const cargoTomlPath = path.join(rootDir, 'src-tauri', 'Cargo.toml');
const cargoLockPath = path.join(rootDir, 'src-tauri', 'Cargo.lock');
const tauriConfigPath = path.join(rootDir, 'src-tauri', 'tauri.conf.json');

function readJson(filePath) {
  return JSON.parse(readFileSync(filePath, 'utf8'));
}

function writeJson(filePath, value) {
  writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`);
}

function syncCargoVersion(filePath, version) {
  const source = readFileSync(filePath, 'utf8');
  const next = source.replace(/^version = ".*"$/m, `version = "${version}"`);
  if (next === source) {
    return false;
  }
  writeFileSync(filePath, next);
  return true;
}

function syncJsonVersion(filePath, version) {
  const data = readJson(filePath);
  if (data.version === version) {
    return false;
  }
  data.version = version;
  writeJson(filePath, data);
  return true;
}

function syncCargoLockVersion(filePath, version) {
  const manifest = readFileSync(cargoTomlPath, 'utf8');
  const name = manifest.match(/^name = "([^"]+)"$/m)?.[1];
  const source = readFileSync(filePath, 'utf8');
  let found = false;
  const next = source.replace(/(\[\[package\]\]\r?\nname = "([^"]+)"\r?\nversion = ")[^"]+("(?:\r?\n|$))/g,
    (entry, prefix, packageName, suffix) => {
      if (packageName !== name) return entry;
      found = true;
      return `${prefix}${version}${suffix}`;
    });
  if (!found) throw new Error(`Could not find ${name} in Cargo.lock`);
  if (next === source) return false;
  writeFileSync(filePath, next);
  return true;
}

const packageJson = readJson(packageJsonPath);
const version = packageJson.version;

if (typeof version !== 'string' || version.trim() === '') {
  console.error('package.json is missing a valid version string');
  process.exit(1);
}

const changed = [];

if (syncCargoVersion(cargoTomlPath, version)) {
  changed.push(path.relative(rootDir, cargoTomlPath));
}

if (syncCargoLockVersion(cargoLockPath, version)) {
  changed.push(path.relative(rootDir, cargoLockPath));
}

if (syncJsonVersion(tauriConfigPath, version)) {
  changed.push(path.relative(rootDir, tauriConfigPath));
}

if (changed.length > 0) {
  console.log(`Synced version ${version} -> ${changed.join(', ')}`);
} else {
  console.log(`Version ${version} already synced`);
}
