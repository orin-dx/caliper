// An independent reference for type correctness (docs/measurement.md, "Independent reference"). The checker oracle
// (src/oracle.ts) scores a tool against the TypeScript checker; this scores it against the checker AND the type each
// declaration writes (src/reference-declared.ts), so an experimental checker-backed mode is not judged by the same
// checker alone, and the props where the two disagree go to a person.
//
//   node src/reference.ts --config FILE
//
// The config is JSON: { corpora: [{ name, root, tsconfig, files, caliper? }], list?, perCorpus?, seed?, universeCap?,
// out, review? }. `files` is a file of component file paths, one per line (the list src/oracle.ts takes); `caliper` is
// that corpus's `extract --json`, when a tool is to be scored; `list` is a JSON array of { corpus, file, component, prop }
// to judge besides the control sample (for example the props two modes differ on).
import { readFileSync, writeFileSync } from 'node:fs'
import { API } from 'typescript-beta/sync'
import { checkerNormalizer } from './oracle-checker.ts'
import type { Norm } from './oracle-compare.ts'
import { declaredTypeOfAll } from './reference-declared.ts'
import { reference, score, type Reference } from './reference-judge.ts'
import { informative, reviewText, summarize, type Row } from './reference-report.ts'
import { keyOf, stratified, stratumOf, type Candidate } from './reference-sample.ts'
import { loadTool, type Tool } from './reference-tool.ts'
import { collect, normComponent, type Entry, type Universe } from './reference-universe.ts'

interface CorpusConfig {
  name: string
  root: string
  tsconfig: string
  files: string
  caliper?: string
}
interface Config {
  corpora: CorpusConfig[]
  list?: string
  perCorpus?: number
  seed?: string
  universeCap?: number
  /** Draw the control sample from every component the checker finds, not only the ones the tool reports. */
  allComponents?: boolean
  out: string
  review?: string
  /** Every judged row as JSON, for looking at single props; defaults to `<out>.rows.json`. */
  rows?: string
}
interface Listed {
  corpus: string
  file: string
  component: string
  prop: string
}

const DEFAULT_PER_CORPUS = 100
const DEFAULT_CAP = 8000

function readConfig(): Config {
  const at = process.argv.indexOf('--config')
  if (at === -1 || !process.argv[at + 1]) {
    console.error('usage: node src/reference.ts --config FILE')
    process.exit(2)
  }
  return JSON.parse(readFileSync(process.argv[at + 1], 'utf8'))
}

/** The declaration of each of a prop's symbols as source text; a declaration the project cannot resolve is left out. */
function declarationTexts(entry: Entry, project: any): string[] {
  const texts: string[] = []
  for (const declaration of entry.symbol.declarations ?? []) {
    try {
      texts.push(declaration.resolve(project).getText())
    } catch {
      // A declaration in a file the program did not load: method B has nothing to read.
    }
  }
  return texts
}

/** What judging a prop needs besides the prop: the program, the tool's output, and the checker's normalizer. */
interface Context {
  universe: Universe
  tool?: Tool
  norm: (t: any) => Norm
}

function judge(entry: Entry, { universe, tool, norm }: Context): Row {
  const { candidate } = entry
  const checkerNorm = norm(entry.type)
  const declared = declaredTypeOfAll(declarationTexts(entry, universe.project))
  const ref: Reference = reference(checkerNorm, declared.norm)
  const row: Row = {
    corpus: candidate.corpus,
    stratum: stratumOf(candidate, entry.listed),
    origin: candidate.origin,
    file: candidate.file,
    component: candidate.component,
    prop: candidate.prop,
    checker: universe.checker.typeToString(entry.type),
    declared: declared.text,
    abstained: declared.abstained,
    reference: ref.kind,
    location: entry.symbol.declarations?.[0]?.path,
  }
  if (ref.kind === 'agreed') Object.assign(row, { basis: ref.basis, why: ref.why })
  if (ref.kind === 'contested') row.pair = `${ref.declared.k}->${ref.checker.k}`
  const found = tool?.lookup(candidate.file, candidate.component, candidate.prop)
  if (found?.found) row.tool = score(found.norm, ref, found.layered)
  return row
}

/** The props to judge: every listed one the checker found, and the stratified control sample over the rest. */
function choose(universe: Universe, perCorpus: number, seed: string): Entry[] {
  const listed = universe.entries.filter((e) => e.listed)
  const pool = universe.entries.filter((e) => !e.listed)
  const byKey = new Map(pool.map((e) => [keyOf(e.candidate), e]))
  const sample = stratified(
    pool.map((e) => e.candidate),
    perCorpus,
    seed,
  )
  return [...listed, ...sample.map((c: Candidate) => byKey.get(keyOf(c))!)]
}

function runCorpus(corpus: CorpusConfig, config: Config, listed: Listed[]): Row[] {
  const api = new API({ cwd: corpus.root })
  const { options } = api.parseConfigFile(corpus.tsconfig)
  const files = readFileSync(corpus.files, 'utf8').split('\n').filter(Boolean)
  const program = api.createProgram(files, options)
  const project = program.getProject()
  const wanted = new Set(
    listed.filter((l) => l.corpus === corpus.name).map((l) => keyOf({ ...l, component: normComponent(l.component) })),
  )
  const seed = config.seed ?? 'caliper'
  const tool = corpus.caliper ? loadTool(corpus.caliper, corpus.root) : undefined
  const universe = collect(files, {
    checker: project.checker,
    project,
    corpus: corpus.name,
    root: corpus.root,
    wanted,
    cap: config.universeCap ?? DEFAULT_CAP,
    seed,
    accept: tool && !config.allComponents ? (file, component) => tool.has(file, component) : undefined,
  })
  const context: Context = { universe, tool, norm: checkerNormalizer(project.checker) }
  const rows = choose(universe, config.perCorpus ?? DEFAULT_PER_CORPUS, `${seed}:${corpus.name}`).map((entry) =>
    judge(entry, context),
  )
  console.error(
    `${corpus.name}: ${universe.components} components, ${universe.entries.length} candidate props, ${rows.length} judged`,
  )
  program.dispose()
  api.close()
  return rows
}

const config = readConfig()
const listed: Listed[] = config.list ? JSON.parse(readFileSync(config.list, 'utf8')) : []
const report: Record<string, unknown> = {
  seed: config.seed ?? 'caliper',
  per_corpus: config.perCorpus ?? DEFAULT_PER_CORPUS,
}
const allRows: Row[] = []
for (const corpus of config.corpora) {
  const rows = runCorpus(corpus, config, listed)
  allRows.push(...rows)
  report[corpus.name] = summarize(rows, Boolean(corpus.caliper))
}
report.informative = informative(allRows)
writeFileSync(config.out, JSON.stringify(report, null, 2))
writeFileSync(config.review ?? `${config.out}.review.md`, reviewText(allRows))
writeFileSync(config.rows ?? `${config.out}.rows.json`, JSON.stringify(allRows, null, 1))
