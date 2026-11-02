// react-docgen (what Storybook ships) on one corpus (scripts/weekly.py): every component's props, in the shape
// scripts/rdt.mjs writes, plus its own wall time and peak RSS. It parses each file with its default resolver and
// importer, as Storybook does: the importer follows imports to the file that declares a type, but nothing is inferred.
// Each prop keeps its `tsType` (without the signature tree) for apps/oracle/src/tool-norm.ts; a file react-docgen throws
// on is counted in `failed` and named in `failedFiles`.
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
const failedFiles = []
const start = performance.now()
// The type without the parameter and member trees a signature carries: the oracle reads the `raw` source text.
const slim = (t) =>
  t && typeof t === 'object'
    ? Object.fromEntries(
        Object.entries(t)
          .filter(([key]) => key !== 'signature')
          .map(([key, v]) => [key, Array.isArray(v) ? v.map(slim) : v]),
      )
    : t
for (const file of files) {
  try {
    for (const doc of parse(readFileSync(file, 'utf8'), { filename: file })) {
      components.push({
        file,
        name: doc.displayName ?? '',
        props: Object.fromEntries(
          Object.entries(doc.props ?? {}).map(([k, p]) => [
            k,
            {
              required: Boolean(p.required),
              type: p.tsType?.raw ?? p.tsType?.name ?? '',
              parent: '',
              tsType: slim(p.tsType),
            },
          ]),
        ),
      })
    }
  } catch {
    failedFiles.push(file)
  }
}
const failed = failedFiles.length
const wallMs = performance.now() - start
// `maxRSS` is in kilobytes on every platform Node supports.
const peakRssMiB = process.resourceUsage().maxRSS / 1024
writeFileSync(out, JSON.stringify({ wallMs, peakRssMiB, failed, failedFiles, components }))
console.error(
  `${components.length} components from ${files.length} files (${failed} failed) in ${(wallMs / 1000).toFixed(1)} s`,
)
