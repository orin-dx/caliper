// react-docgen-typescript on one corpus (scripts/weekly.py): every component's props with their declaring file, plus
// RDT's own wall time and peak RSS. RDT comes from apps/validate, with the options its harness uses.
//
// Usage: node scripts/rdt.mjs <file-list> <tsconfig> <out.json>
import { readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const require = createRequire(fileURLToPath(new URL('../apps/validate/package.json', import.meta.url)))
const { withCustomConfig } = require('react-docgen-typescript')

const [listFile, tsconfig, out] = process.argv.slice(2)
if (!out) {
  console.error('usage: node scripts/rdt.mjs <file-list> <tsconfig> <out.json>')
  process.exit(2)
}
const files = readFileSync(listFile, 'utf8').split('\n').filter(Boolean)
const start = performance.now()
const parser = withCustomConfig(tsconfig, {
  shouldExtractLiteralValuesFromEnum: true,
  shouldRemoveUndefinedFromOptional: true,
  propFilter: { skipPropsWithoutDoc: false },
})
const docs = parser.parse(files)
const wallMs = performance.now() - start
const components = docs.map((d) => ({
  file: d.filePath,
  name: d.displayName,
  props: Object.fromEntries(
    Object.entries(d.props ?? {}).map(([k, p]) => [
      k,
      {
        required: Boolean(p.required),
        type: p.type?.name ?? '',
        parent: p.parent?.fileName ?? p.declarations?.[0]?.fileName ?? '',
      },
    ]),
  ),
}))
// `maxRSS` is in kilobytes on every platform Node supports.
const peakRssMiB = process.resourceUsage().maxRSS / 1024
writeFileSync(out, JSON.stringify({ wallMs, peakRssMiB, components }))
console.error(`dumped ${components.length} components in ${(wallMs / 1000).toFixed(1)} s, ${peakRssMiB.toFixed(0)} MiB`)
