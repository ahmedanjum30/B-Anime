// Writes a version into every file that carries the app's version, without committing.
// The release workflow runs it with the tag's version just before building, so the tag alone
// decides a release's version (the files in the repository aren't bumped).
//
//   node scripts/set-version.mjs 0.3.1
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const version = process.argv[2]?.replace(/^v/, "");
if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  console.error(`Usage: node scripts/set-version.mjs <x.y.z> (got "${process.argv[2] ?? ""}")`);
  process.exit(1);
}
const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function edit(path, change) {
  const file = join(root, path);
  const before = readFileSync(file, "utf8");
  const after = change(before);
  if (after === before) throw new Error(`No version found to set in ${path}`);
  writeFileSync(file, after);
  console.log(`${path}: ${version}`);
}

// The workspace version, which both crates inherit.
edit("Cargo.toml", (t) => t.replace(/^(version = ")[^"]+(")/m, `$1${version}$2`));
// The two workspace crates' entries in the lockfile.
edit("Cargo.lock", (t) =>
  t.replace(/(name = "banime-(?:app|core)"\r?\nversion = ")[^"]+(")/g, `$1${version}$2`),
);
// Tauri reads the app's version (and Android's versionCode) from here.
edit("app/src-tauri/tauri.conf.json", (t) => t.replace(/("version": ")[^"]+(")/, `$1${version}$2`));
edit("app/package.json", (t) => t.replace(/("version": ")[^"]+(")/, `$1${version}$2`));
// package-lock: the top-level version and the root package's.
edit("app/package-lock.json", (t) => {
  const lock = JSON.parse(t);
  lock.version = version;
  if (lock.packages?.[""]) lock.packages[""].version = version;
  return JSON.stringify(lock, null, 2) + "\n";
});
