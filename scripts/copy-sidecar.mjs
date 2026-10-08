#!/usr/bin/env node
// Build rekindle-server and copy to src-tauri/binaries/ with target-triple suffix.
// Tauri's externalBin expects: src-tauri/binaries/<name>-<target-triple>[.exe]
// Usage: node scripts/copy-sidecar.mjs [--release]
import { execSync } from 'child_process';
import { copyFileSync, mkdirSync } from 'fs';
import { join } from 'path';

const isRelease = process.argv.includes('--release');
const profile = isRelease ? 'release' : 'debug';
const cargoFlag = isRelease ? ' --release' : '';

console.log(`Building rekindle-server (${profile})...`);
execSync(`cargo build -p rekindle-server${cargoFlag}`, { stdio: 'inherit' });

// Get host target triple from rustc
const rustcV = execSync('rustc -vV', { encoding: 'utf8' });
const triple = rustcV.match(/host: (.+)/)[1].trim();

mkdirSync(join('src-tauri', 'binaries'), { recursive: true });

const ext = process.platform === 'win32' ? '.exe' : '';
const src = join('target', profile, `rekindle-server${ext}`);
const dst = join('src-tauri', 'binaries', `rekindle-server-${triple}${ext}`);
copyFileSync(src, dst);
console.log(`Copied ${src} → ${dst}`);
