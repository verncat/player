import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

const artifactsDir = resolve(process.env.UPDATER_ARTIFACTS_DIR || "release-artifacts");
const outputPath = resolve(process.env.UPDATER_MANIFEST_PATH || join(artifactsDir, "latest.json"));
const repository = process.env.GITHUB_REPOSITORY?.trim();
const releaseTag = process.env.RELEASE_TAG?.trim();
const notesPath = process.env.RELEASE_NOTES_PATH?.trim();

if (!repository || !releaseTag) {
  throw new Error("GITHUB_REPOSITORY and RELEASE_TAG are required");
}

function walkFiles(directory) {
  return readdirSync(directory).flatMap((entry) => {
    const path = join(directory, entry);
    return statSync(path).isDirectory() ? walkFiles(path) : [path];
  });
}

function updaterTargets(assetName) {
  const lower = assetName.toLowerCase();
  if (lower.endsWith(".app.tar.gz")) {
    return ["darwin-x86_64", "darwin-aarch64"];
  }
  if (lower.endsWith(".appimage")) {
    return [lower.includes("aarch64") || lower.includes("arm64")
      ? "linux-aarch64"
      : "linux-x86_64"];
  }
  if (lower.endsWith("-setup.exe") || lower.endsWith(".msi")) {
    return [lower.includes("aarch64") || lower.includes("arm64")
      ? "windows-aarch64"
      : "windows-x86_64"];
  }
  return [];
}

function assetPriority(assetName) {
  const lower = assetName.toLowerCase();
  if (lower.endsWith("-setup.exe")) return 2;
  if (lower.endsWith(".msi")) return 1;
  return 1;
}

const candidates = new Map();
for (const signaturePath of walkFiles(artifactsDir).filter((path) => path.endsWith(".sig"))) {
  const assetPath = signaturePath.slice(0, -4);
  if (!existsSync(assetPath)) continue;

  const assetName = basename(assetPath);
  const signature = readFileSync(signaturePath, "utf8").trim();
  const url = `https://github.com/${repository}/releases/download/${releaseTag}/${encodeURIComponent(assetName)}`;
  const priority = assetPriority(assetName);

  for (const target of updaterTargets(assetName)) {
    const current = candidates.get(target);
    if (!current || priority > current.priority) {
      candidates.set(target, { priority, signature, url });
    }
  }
}

const requiredTargets = [
  "linux-x86_64",
  "windows-x86_64",
  "darwin-x86_64",
  "darwin-aarch64",
];
const missingTargets = requiredTargets.filter((target) => !candidates.has(target));
if (missingTargets.length > 0) {
  throw new Error(`Missing signed updater artifacts for: ${missingTargets.join(", ")}`);
}

const platforms = Object.fromEntries(
  [...candidates.entries()].map(([target, { signature, url }]) => [target, { signature, url }]),
);
const notes = notesPath && existsSync(notesPath)
  ? readFileSync(notesPath, "utf8").trim()
  : "";

writeFileSync(outputPath, `${JSON.stringify({
  version: releaseTag.replace(/^[vV]/, ""),
  notes,
  pub_date: new Date().toISOString(),
  platforms,
}, null, 2)}\n`);

console.log(`Generated ${outputPath} for ${Object.keys(platforms).join(", ")}`);
