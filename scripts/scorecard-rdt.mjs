#!/usr/bin/env node
// Time react-docgen-typescript on one source directory, the way scripts/scorecard.py times Caliper: every `.ts`/`.tsx`
// file Caliper would discover, parsed as one program. Prints {"ms", "components", "props"} as JSON.
//
// Usage: node scripts/scorecard-rdt.mjs SRC_DIR
// Resolves react-docgen-typescript and typescript from apps/validate, so run `pnpm install` first.
import { readdirSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'apps/validate/package.json'))
const { withCompilerOptions } = require('react-docgen-typescript')
const ts = require('typescript')

// The same exclusions as crates/core/src/pipeline/discover.rs.
function excluded(relative) {
  const name = path.basename(relative)
  const parts = relative.split(path.sep)
  return (
    name.includes('.stories.') ||
    name.includes('.test.') ||
    name.includes('.spec.') ||
    parts.includes('__snapshots__') ||
    parts.includes('node_modules')
  )
}

function discover(dir) {
  return readdirSync(dir, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile() && /\.tsx?$/.test(entry.name))
    .map((entry) => path.join(entry.parentPath, entry.name))
    .filter((file) => !excluded(path.relative(dir, file)))
    .sort()
}

const src = path.resolve(process.argv[2])
const files = discover(src)
const start = performance.now()
const parser = withCompilerOptions(
  { jsx: ts.JsxEmit.ReactJSX, esModuleInterop: true, skipLibCheck: true, noEmit: true },
  { shouldExtractLiteralValuesFromEnum: true, shouldRemoveUndefinedFromOptional: true },
)
const components = parser.parse(files)
const ms = performance.now() - start
const props = components.reduce((sum, component) => sum + Object.keys(component.props ?? {}).length, 0)
console.log(JSON.stringify({ ms: Math.round(ms), components: components.length, props }))
