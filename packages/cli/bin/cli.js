#!/usr/bin/env node
// Thin wrapper — finds the right platform binary and execs it, forwarding argv/stdio/exit
// code untouched. All actual CLI behavior (arg parsing, output formatting, RDT/Storybook
// serialization, exit codes) lives in crates/cli. This file must never grow logic that
// duplicates it — see packages/napi/index.js for the same pattern applied to the native
// addon.
'use strict'

const { existsSync } = require('node:fs')
const { join } = require('node:path')
const { spawnSync } = require('node:child_process')

const BINARY_NAME = process.platform === 'win32' ? 'caliper-docgen.exe' : 'caliper-docgen'

function platformPackageName() {
  const { platform, arch } = process
  if (platform === 'darwin') {
    if (arch === 'arm64') return 'caliper-docgen-darwin-arm64'
    if (arch === 'x64') return 'caliper-docgen-darwin-x64'
  }
  if (platform === 'win32' && arch === 'x64') return 'caliper-docgen-win32-x64-msvc'
  if (platform === 'linux') {
    const isMusl = !process.report?.getReport()?.header?.glibcVersionRuntime
    if (arch === 'x64' && !isMusl) return 'caliper-docgen-linux-x64-gnu'
    if (arch === 'arm64' && !isMusl) return 'caliper-docgen-linux-arm64-gnu'
  }
  return null
}

function findBinary() {
  // Dev-local: this monorepo's own cargo build output, checked first so contributors
  // building from source don't need the platform package published at all.
  const devCandidates = [
    join(__dirname, '../../../target/release', BINARY_NAME),
    join(__dirname, '../../../target/debug', BINARY_NAME),
  ]
  const devFound = devCandidates.find((p) => existsSync(p))
  if (devFound) return devFound

  const pkg = platformPackageName()
  if (!pkg) return null
  try {
    // The platform package's own entry point is the binary path itself (see its package.json).
    return require.resolve(`${pkg}/${BINARY_NAME}`)
  } catch {
    return null
  }
}

const binary = findBinary()
if (!binary) {
  const pkg = platformPackageName()
  process.stderr.write(
    `caliper-docgen: native binary not found.\n${
      pkg
        ? `Expected it via the optional dependency '${pkg}' — make sure your package manager installs ` +
          `optionalDependencies for your platform (${process.platform}-${process.arch}), or run ` +
          `'pnpm run build:napi' (from packages/napi) if developing this repo directly.\n`
        : `Unsupported platform: ${process.platform}-${process.arch}.\n`
    }`,
  )
  process.exit(1)
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' })
if (result.error) {
  process.stderr.write(`caliper-docgen: failed to run native binary: ${result.error.message}\n`)
  process.exit(1)
}
process.exit(result.status ?? 1)
