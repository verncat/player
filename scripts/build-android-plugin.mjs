import { spawnSync } from 'node:child_process';
import { readdirSync, existsSync, mkdirSync, copyFileSync } from 'node:fs';
import path from 'node:path';

const folder = path.resolve(process.argv[2] || 'plugins/examples/android-native');
const abis = (process.argv[3] || 'arm64-v8a').split(',');
const sdk = process.env.ANDROID_HOME || process.env.ANDROID_SDK_ROOT;
if (!sdk) throw new Error('Set ANDROID_HOME to the Android SDK directory');
const ndk = process.env.ANDROID_NDK_HOME || path.join(sdk, 'ndk',
  readdirSync(path.join(sdk, 'ndk')).sort((a, b) => a.localeCompare(b, undefined, { numeric: true })).at(-1));
const host = { darwin: 'darwin-x86_64', linux: 'linux-x86_64', win32: 'windows-x86_64' }[process.platform];
if (!host) throw new Error(`Unsupported NDK build host: ${process.platform}`);
const targets = {
  'arm64-v8a': ['aarch64-linux-android', 'aarch64-linux-android'],
  'armeabi-v7a': ['armv7-linux-androideabi', 'armv7a-linux-androideabi'],
  x86_64: ['x86_64-linux-android', 'x86_64-linux-android'],
  x86: ['i686-linux-android', 'i686-linux-android'],
};
for (const abi of abis) {
  const target = targets[abi];
  if (!target) throw new Error(`Unsupported Android ABI: ${abi}`);
  const [rustTarget, clangTarget] = target;
  const linker = path.join(ndk, 'toolchains/llvm/prebuilt', host, 'bin', `${clangTarget}26-clang${process.platform === 'win32' ? '.cmd' : ''}`);
  if (!existsSync(linker)) throw new Error(`NDK compiler not found: ${linker}`);
  const output = path.join(folder, 'native/target');
  const result = spawnSync('cargo', ['build', '--locked', '--release', '--manifest-path', path.join(folder, 'native/Cargo.toml'), '--target', rustTarget], {
    stdio: 'inherit', env: { ...process.env, CARGO_TARGET_DIR: output,
      [`CARGO_TARGET_${rustTarget.toUpperCase().replaceAll('-', '_')}_LINKER`]: linker },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status || 1);
  const destination = path.join(folder, 'build/generated/jniLibs', abi);
  mkdirSync(destination, { recursive: true });
  copyFileSync(path.join(output, rustTarget, 'release/libplayer_native_example.so'), path.join(destination, 'libplayer_native_example.so'));
}
