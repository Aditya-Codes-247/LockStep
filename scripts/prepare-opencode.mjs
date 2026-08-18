#!/usr/bin/env node
// Stages the opencode CLI binary for bundling with the Tauri app.
//
// The `opencode-ai` npm package downloads the correct platform binary into
// `node_modules/opencode-ai/bin/opencode(.exe)` during `npm install` (its
// postinstall resolves the platform-specific optional dependency). This
// script copies that binary to `src-tauri/binaries/opencode-<target-triple>[.exe]`,
// which Tauri's `bundle.externalBin` picks up and ships inside the app bundle
// at `resource_dir()/bin/opencode(.exe)`.
//
// Override the target triple with OPENCODE_TARGET_TRIPLE when cross-compiling.

import { execSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");
const binariesDir = path.join(repoRoot, "src-tauri", "binaries");

const isWindows = os.platform() === "win32";

function fail(message) {
  console.error(`\n[prepare-opencode] ERROR: ${message}\n`);
  process.exit(1);
}

function hostTriple() {
  const override = process.env.OPENCODE_TARGET_TRIPLE;
  if (override) return override.trim();

  try {
    const out = execSync("rustc -vV", { encoding: "utf8" });
    const match = out.match(/host:\s*(\S+)/);
    if (match) return match[1];
  } catch {
    // fall through to the platform map below
  }

  const platformMap = { win32: "windows", darwin: "darwin", linux: "linux" };
  const archMap = { x64: "x86_64", arm64: "aarch64" };
  const osName = platformMap[os.platform()];
  const arch = archMap[os.arch()];
  if (osName && arch) {
    if (osName === "windows") return `${arch}-pc-windows-msvc`;
    if (osName === "darwin") return `${arch}-apple-darwin`;
    if (osName === "linux") return `${arch}-unknown-linux-gnu`;
  }
  fail("Could not determine the Rust target triple. Set OPENCODE_TARGET_TRIPLE to override.");
}

function platformPackageName() {
  const platformMap = { win32: "windows", darwin: "darwin", linux: "linux" };
  const archMap = { x64: "x64", arm64: "arm64" };
  const osName = platformMap[os.platform()];
  const arch = archMap[os.arch()];
  if (!osName || !arch) return null;
  return `opencode-${osName}-${arch}`;
}

function findSourceBinary() {
  const candidates = [];
  const binName = isWindows ? "opencode.exe" : "opencode";

  // 1) The opencode-ai postinstall copies the resolved platform binary here
  //    (it always writes `bin/opencode.exe`, even on Unix).
  candidates.push(path.join(repoRoot, "node_modules", "opencode-ai", "bin", "opencode.exe"));
  candidates.push(path.join(repoRoot, "node_modules", "opencode-ai", "bin", binName));

  // 2) Platform-specific optional dependency, in case scripts were disabled.
  const pkg = platformPackageName();
  if (pkg) {
    candidates.push(path.join(repoRoot, "node_modules", pkg, "bin", binName));
    candidates.push(path.join(repoRoot, "node_modules", pkg, "bin", "opencode.exe"));
  }

  for (const candidate of candidates) {
    try {
      if (fs.statSync(candidate).isFile()) return candidate;
    } catch {
      // not present, try the next candidate
    }
  }
  return null;
}

function isWindowsTriple(triple) {
  return /windows/i.test(triple);
}

function main() {
  const triple = hostTriple();
  const exeSuffix = isWindowsTriple(triple) ? ".exe" : "";
  const destination = path.join(binariesDir, `opencode-${triple}${exeSuffix}`);

  const source = findSourceBinary();
  if (!source) {
    fail(
      "opencode binary not found in node_modules. Run `npm install` first so the opencode-ai\n" +
        "postinstall can download the platform binary, then re-run `npm run prepare:opencode`.",
    );
  }

  fs.mkdirSync(binariesDir, { recursive: true });
  fs.copyFileSync(source, destination);
  try {
    fs.chmodSync(destination, 0o755);
  } catch {
    // chmod is a no-op on Windows; ignore.
  }

  const verify = spawnSync(destination, ["--version"], { encoding: "utf8", timeout: 30_000 });
  const version = (verify.stdout || verify.stderr || "").trim().split("\n")[0];

  console.log(`[prepare-opencode] staged ${destination}`);
  console.log(`[prepare-opencode] version: ${version || "(unknown)"}`);
  if (verify.status !== 0) {
    fail(`staged binary failed --version (status ${verify.status}). ${verify.stderr || ""}`);
  }
}

main();
