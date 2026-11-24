// Writes a version into every file that carries the app's version, without committing.
// The release workflow runs it with the tag's version just before building, so the tag alone
// decides a release's version (the files in the repository aren't bumped).
//
//   node scripts/set-version.mjs 0.3.1
//   node scripts/set-version.mjs 0.4.5-beta.2   (the 2nd beta of 0.4.5)
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const version = process.argv[2]?.replace(/^v/, "");
const parsed = /^(\d+)\.(\d+)\.(\d+)(?:-beta\.(\d+))?$/.exec(version ?? "");
if (!parsed || (parsed[4] !== undefined && !(Number(parsed[4]) >= 1 && Number(parsed[4]) <= 98))) {
  console.error(`Usage: node scripts/set-version.mjs <x.y.z | x.y.z-beta.N, N 1–98> (got "${process.argv[2] ?? ""}")`);
  process.exit(1);
}
const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * Sets the version in one file. Fails only when the version field isn't there at all: the file
 * already holding this version (a manual run on a branch stamps the files' own version) is fine.
 */
function edit(path, pattern, change) {
  const file = join(root, path);
  const before = readFileSync(file, "utf8");
  if (pattern && !new RegExp(pattern.source, pattern.flags.replace("g", "")).test(before)) {
    throw new Error(`No version found to set in ${path}`);
  }
  const after = change(before);
  if (after !== before) writeFileSync(file, after);
  console.log(`${path}: ${version}${after === before ? " (already)" : ""}`);
}

// The workspace version, which both crates inherit.
const cargoToml = /^(version = ")[^"]+(")/m;
edit("Cargo.toml", cargoToml, (t) => t.replace(cargoToml, `$1${version}$2`));
// The two workspace crates' entries in the lockfile.
const cargoLock = /(name = "banime-(?:app|core)"\r?\nversion = ")[^"]+(")/g;
edit("Cargo.lock", cargoLock, (t) => t.replace(cargoLock, `$1${version}$2`));
// Tauri reads the app's version (and Android's versionCode) from here.
const jsonVersion = /("version": ")[^"]+(")/;
edit("app/src-tauri/tauri.conf.json", jsonVersion, (t) => t.replace(jsonVersion, `$1${version}$2`));
edit("app/package.json", jsonVersion, (t) => t.replace(jsonVersion, `$1${version}$2`));
// package-lock: the top-level version and the root package's.
edit("app/package-lock.json", null, (t) => {
  const lock = JSON.parse(t);
  lock.version = version;
  if (lock.packages?.[""]) lock.packages[""].version = version;
  return JSON.stringify(lock, null, 2) + "\n";
});

// Android's versionCode must rise with every release, betas included, and a final release must
// rank above its betas: (x·1,000,000 + y·1,000 + z)·100 + N for beta N, + 99 for the release.
// (Before betas, Tauri used x·1,000,000 + y·1,000 + z: 0.4.4 was 4004, far below 400501.)
const [, major, minor, patch, beta] = parsed;
const versionCode = (Number(major) * 1_000_000 + Number(minor) * 1_000 + Number(patch)) * 100 + (beta ? Number(beta) : 99);
edit("app/src-tauri/tauri.conf.json", null, (t) => {
  const conf = JSON.parse(t);
  conf.bundle ??= {};
  conf.bundle.android = { ...conf.bundle.android, versionCode };
  return JSON.stringify(conf, null, 2) + "\n";
});
console.log(`Android versionCode: ${versionCode}`);
