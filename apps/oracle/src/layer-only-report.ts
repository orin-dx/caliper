// The layer-only props of each corpus, sorted into API and ambient (src/layer-only.ts, docs/measurement.md, "Layer-only
// props"). Reads the canonical JSON of two runs per corpus and writes one report.
//
//   node src/layer-only-report.ts --config FILE
//
// The config is JSON: { corpora: [{ name, default, expanded? }], top?, out }. `default` is `extract --json` at the default
// collapse and `expanded` the same with `--collapse-inherited-props none`.
import { readFileSync, statSync, writeFileSync } from 'node:fs'
import { readFile } from './layer-only.ts'
import { analyse } from './layer-only-stats.ts'

interface Config {
  corpora: { name: string; default: string; expanded?: string }[]
  top?: number
  out: string
}

const at = process.argv.indexOf('--config')
if (at === -1 || !process.argv[at + 1]) {
  console.error('usage: node src/layer-only-report.ts --config FILE')
  process.exit(2)
}
const config: Config = JSON.parse(readFileSync(process.argv[at + 1], 'utf8'))
const report: Record<string, unknown> = {}
for (const corpus of config.corpora) {
  const expanded = corpus.expanded ? JSON.parse(readFileSync(corpus.expanded, 'utf8')) : undefined
  const bytes = corpus.expanded
    ? { default: statSync(corpus.default).size, expanded: statSync(corpus.expanded).size }
    : undefined
  const input = { name: corpus.name, caliper: JSON.parse(readFileSync(corpus.default, 'utf8')), expanded, bytes }
  report[corpus.name] = analyse(input, readFile, config.top ?? 20)
}
writeFileSync(config.out, JSON.stringify(report, null, 2))
