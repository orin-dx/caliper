#!/usr/bin/env node
// `napi build` writes the package version into every binding check of packages/napi/index.js as a literal. The version
// PR bumps package.json but not that generated file, so the literal would go stale. Rewriting it to read the version of
// the package that ships the loader keeps the committed file valid across version bumps.
// Run after `napi build`; `scripts/check-packaging.mjs` fails if a literal is left.
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const loaderPath = join(dirname(fileURLToPath(import.meta.url)), '..', 'packages/napi/index.js')
const version = "require('./package.json').version"
const source = readFileSync(loaderPath, 'utf8')
const rewritten = source
  .replace(/bindingPackageVersion !== '[^']+'/g, `bindingPackageVersion !== ${version}`)
  .replace(/expected [^\s$]+ but got \$\{bindingPackageVersion\}/g, `expected \${${version}} but got \${bindingPackageVersion}`)
if (rewritten !== source) writeFileSync(loaderPath, rewritten)
