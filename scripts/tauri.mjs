import { spawn } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Keep proprietary SDK binaries outside the repository. Bundle only when explicitly configured.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const sdk = process.env.DISCORD_SOCIAL_SDK_PATH;
const env = { ...process.env };
const socialEnabled = !args.includes('--no-default-features') || args.some(arg => arg.includes('discord-social'));
if (sdk && socialEnabled && ['dev', 'build', 'bundle'].includes(args[0])) {
  const targetIndex = args.indexOf('--target');
  const target = targetIndex >= 0 ? args[targetIndex + 1] : args.find(arg => arg.startsWith('--target='))?.split('=')[1];
  const platform = target?.includes('windows') ? 'win32' : target?.includes('linux') ? 'linux' : target?.includes('darwin') ? 'darwin' : process.platform;
  const sdkRoot = path.resolve(sdk);
  env.DISCORD_SOCIAL_SDK_PATH = sdkRoot;
  const lib = path.join(sdkRoot, 'lib', 'release');
  const check = file => { if (!existsSync(file)) throw new Error(`Discord SDK runtime missing: ${file}`); return file; };
  let config;
  if (platform === 'darwin') {
    const base = JSON.parse(readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
    const identity = env.APPLE_SIGNING_IDENTITY || base.bundle?.macOS?.signingIdentity || '-';
    // Bundling changes dylib install names and invalidates their original signatures.
    // Supply the fallback first so explicit CLI signing configurations still take precedence.
    args.splice(1, 0, '--config', JSON.stringify({ bundle: { macOS: { signingIdentity: identity } } }));
    config = { bundle: { macOS: { frameworks: [check(path.join(lib, 'libdiscord_partner_sdk.dylib')), check(path.join(lib, 'libdiscord_krisp.dylib'))] } } };
  } else if (platform === 'win32') {
    const arm = target?.startsWith('aarch64') || (!target && process.arch === 'arm64');
    const bin = path.join(sdkRoot, 'bin', 'release', ...(arm ? ['arm64'] : []));
    config = { bundle: { resources: Object.fromEntries(['discord_partner_sdk.dll', 'discord_krisp.dll'].map(name => [check(path.join(bin, name)), name])) } };
    env.PATH = `${bin}${path.delimiter}${env.PATH ?? ''}`;
  } else if (platform === 'linux') {
    config = { bundle: { resources: { [check(path.join(lib, 'libdiscord_partner_sdk.so'))]: 'libdiscord_partner_sdk.so' } } };
  }
  if (config) { const separator = args.indexOf('--'); args.splice(separator < 0 ? args.length : separator, 0, '--config', JSON.stringify(config)); }
}
const child = spawn(process.execPath, [path.join(root, 'node_modules/@tauri-apps/cli/tauri.js'), ...args], { cwd: root, env, stdio: 'inherit' });
child.on('error', error => { console.error(error.message); process.exitCode = 1; });
child.on('exit', (code, signal) => { if (signal) process.kill(process.pid, signal); else process.exitCode = code ?? 1; });
