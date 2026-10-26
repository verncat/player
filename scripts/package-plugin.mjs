import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

const folder = process.argv[2];
if (!folder) throw new Error('Usage: node scripts/package-plugin.mjs <plugin-folder> [output.player-plugin]');
const manifest = JSON.parse(await readFile(path.join(folder, 'plugin.json'), 'utf8'));
const script = await readFile(path.join(folder, 'main.js'), 'utf8');
const content = JSON.stringify({ manifest, script }, null, 2) + '\n';
if (Buffer.byteLength(content) > 4 * 1024 * 1024) throw new Error('Package exceeds 4 MiB');
const output = process.argv[3] || path.join(folder, `${manifest.id}-${manifest.version}.player-plugin`);
await writeFile(output, content, 'utf8');
console.log(`Created ${output} (${Buffer.byteLength(content)} bytes)`);
