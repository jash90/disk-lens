#!/usr/bin/env node
// Builds latest.json for the in-app updater from a folder of release assets and verifies every
// updater signature against the public key in src-tauri/tauri.conf.json.
// Usage: node scripts/latest-json.mjs <assets-dir> [notes-file]
// Asset names must already use dots instead of spaces (Disk.Lens_…), exactly as GitHub stores them.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const [dir, notesFile] = process.argv.slice(2);
if (!dir) {
  console.error("usage: latest-json.mjs <assets-dir> [notes-file]");
  process.exit(2);
}

const conf = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
const { version } = conf;
const pubkey = conf.plugins.updater.pubkey;
const base = `https://github.com/jash90/disk-lens/releases/download/v${version}`;

// Updater artifact → platform keys (the same keys tauri-action writes).
const RULES = [
  [/_universal\.app\.tar\.gz$/, ["darwin-aarch64", "darwin-aarch64-app", "darwin-x86_64", "darwin-x86_64-app"]],
  [/_x64-setup\.exe$/, ["windows-x86_64", "windows-x86_64-nsis"]],
  [/_arm64-setup\.exe$/, ["windows-aarch64", "windows-aarch64-nsis"]],
  [/_amd64\.AppImage$/, ["linux-x86_64", "linux-x86_64-appimage"]],
  [/_amd64\.deb$/, ["linux-x86_64-deb"]],
  [/\.x86_64\.rpm$/, ["linux-x86_64-rpm"]],
  [/_aarch64\.AppImage$/, ["linux-aarch64", "linux-aarch64-appimage"]],
  [/_arm64\.deb$/, ["linux-aarch64-deb"]],
  [/\.aarch64\.rpm$/, ["linux-aarch64-rpm"]],
];

const platforms = {};
for (const name of readdirSync(dir).sort()) {
  if (!name.includes(`_${version}`) && !name.includes(`-${version}-`)) continue;
  const rule = RULES.find(([re]) => re.test(name));
  if (!rule) continue;
  const sigPath = join(dir, `${name}.sig`);
  if (!existsSync(sigPath)) {
    console.error(`FAIL ${name}: missing ${name}.sig (was TAURI_SIGNING_PRIVATE_KEY set during the build?)`);
    process.exit(1);
  }
  try {
    execFileSync("node", [join(root, "scripts/lib/verify-minisign.mjs"), join(dir, name), sigPath, pubkey], { stdio: "inherit" });
  } catch {
    process.exit(1); // verify-minisign.mjs already printed FAIL with the reason
  }
  // The updater rejects an archive whose signed version differs from latest.json (SignedVersionMismatch).
  const trusted = Buffer.from(readFileSync(sigPath, "utf8").trim(), "base64").toString("utf8");
  const signedVersion = /version:(\S+)/.exec(trusted)?.[1];
  if (signedVersion && signedVersion !== version) {
    console.error(`FAIL ${name}: signed as version ${signedVersion}, release is ${version}`);
    process.exit(1);
  }
  const entry = { signature: readFileSync(sigPath, "utf8").trim(), url: `${base}/${name}` };
  for (const key of rule[1]) platforms[key] = entry;
}

if (Object.keys(platforms).length === 0) {
  console.error(`no updater artifacts for ${version} in ${dir}`);
  process.exit(1);
}
const manifest = {
  version,
  notes: notesFile ? readFileSync(notesFile, "utf8").trim() : "",
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  platforms,
};
writeFileSync(join(dir, "latest.json"), JSON.stringify(manifest, null, 2) + "\n");
console.log(`latest.json ${version}: ${Object.keys(platforms).join(", ")}`);
