// react-docgen (what Storybook ships) on one corpus (scripts/weekly.py): every component's props, in the shape
// scripts/rdt.mjs writes, plus its own wall time and peak RSS. It reads each file on its own with its default
// resolver and importer, as Storybook does, so it knows nothing about types declared in other files.
//
// Usage: node scripts/rdg.mjs <file-list> <out.json>
import { readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { pathToFileURL, fileURLToPath } from 'node:url'

const require = createRequire(fileURLToPath(new URL('../apps/oracle/package.json', import.meta.url)))
const { parse } = await import(pathToFileURL(require.resolve('react-docgen')).href)

const [listFile, out] = process.argv.slice(2)
if (!out) {
  console.error('usage: node scripts/rdg.mjs <file-list> <out.json>')
  process.exit(2)
}
const files = readFileSync(listFile, 'utf8').split('\n').filter(Boolean)
const components = []
let failed = 0
const start = performance.now()
for (const file of files) {
  try {
    for (const doc of parse(readFileSync(file, 'utf8'), { filename: file })) {
      components.push({
        file,
        name: doc.displayName ?? '',
        props: Object.fromEntries(
          Object.entries(doc.props ?? {}).map(([k, p]) => [
            k,
            { required: Boolean(p.required), type: p.tsType?.raw ?? p.tsType?.name ?? '', parent: '' },
          ]),
        ),
      })
    }
  } catch {
    failed += 1
  }
}
const wallMs = performance.now() - start
// `maxRSS` is in kilobytes on every platform Node supports.
const peakRssMiB = process.resourceUsage().maxRSS / 1024
writeFileSync(out, JSON.stringify({ wallMs, peakRssMiB, failed, components }))
console.error(`${components.length} components from ${files.length} files (${failed} failed) in ${(wallMs / 1000).toFixed(1)} s`)
