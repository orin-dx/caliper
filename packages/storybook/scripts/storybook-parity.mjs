#!/usr/bin/env node
// What the addon adds to, and costs against, Storybook's own docgen, on real repositories. See docs/storybook-parity.md.
//
//   node scripts/storybook-parity.mjs --corpora ~/corpora [--only chakra,spectrum] [--out report] [--limit 50]
//
// Needs `pnpm build` (the addon's dist), the native binding, and the corpora cloned with their dependencies installed.
// Each variant runs in its own child process so its peak RSS is its own.
import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { createRequire, registerHooks } from 'node:module'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { parseArgs } from 'node:util'

const here = dirname(fileURLToPath(import.meta.url))
const require = createRequire(import.meta.url)

/** Where each corpus keeps its sources (one directory or several), relative to its root. */
const CORPORA = {
  mantine: { root: 'mantine', src: 'packages/@mantine/core/src' },
  chakra: { root: 'chakra-ui', src: 'packages/react/src' },
  shadcn: { root: 'shadcn-ui', src: 'apps/v4' },
  antd: { root: 'ant-design', src: 'components' },
  spectrum: { root: 'react-spectrum', src: ['packages', 'starters'] },
}

const STORY_FILE = /\.stories\.(tsx?|jsx?)$/

/** The addon variants: what the defaults do, and what the addon did before it read dependencies. */
const VARIANTS = {
  caliper: { loadDependencies: true, collapseInheritedProps: 'none' },
  'caliper-nodeps': { loadDependencies: false, collapseInheritedProps: 'none' },
  'caliper-curated': { loadDependencies: true, collapseInheritedProps: 'dependencies' },
  // The same extractions, answering for every component whatever its diagnostics say: the information the addon
  // holds, apart from how much of it the `degraded` rule lets through.
  'caliper-never': { loadDependencies: true, collapseInheritedProps: 'none', fallback: 'never' },
  'caliper-nodeps-never': { loadDependencies: false, collapseInheritedProps: 'none', fallback: 'never' },
}

/** The session alone does not depend on the fallback rule. */
const SESSION_VARIANTS = ['caliper', 'caliper-nodeps', 'caliper-curated']

/** The lazy session is the default; the eager ones resolve every component at start, as the addon did before it. */
Object.assign(VARIANTS, {
  'caliper-eager': { loadDependencies: true, collapseInheritedProps: 'none', lazy: false },
  'caliper-nodeps-eager': { loadDependencies: false, collapseInheritedProps: 'none', lazy: false },
  // Lazy without the background warm-up of the dependencies every story reaches.
  'caliper-cold': { loadDependencies: true, collapseInheritedProps: 'none', warmUp: false },
})

function walk(dir, found = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'node_modules' || entry.name.startsWith('.git')) continue
    const path = join(dir, entry.name)
    if (entry.isDirectory()) walk(path, found)
    else if (STORY_FILE.test(entry.name)) found.push(path)
  }
  return found
}

/** Node reports `maxRSS` in kilobytes on every platform. */
function peakRssMb() {
  return Math.round(process.resourceUsage().maxRSS / 1024)
}

/** A diagnostic message without the names and paths that make every one unique. */
function shape(message) {
  return message.replaceAll(/'[^']*'/g, "'_'").replaceAll(/\/[^\s]+/g, '<path>')
}

/** One argType reduced to the facts the comparison reads. */
function summarizeArg(arg) {
  const value = arg.type?.value
  return {
    kind: arg.type?.name ?? null,
    options: Array.isArray(value) ? value.map((v) => String(typeof v === 'object' ? (v?.value ?? v?.name) : v)) : null,
    required: Boolean(arg.type?.required),
    type: arg.table?.type?.summary ?? null,
    default: arg.table?.defaultValue?.summary ?? null,
    described: Boolean(arg.description),
  }
}

function summarize(payload) {
  const props = (argTypes) =>
    Object.fromEntries(Object.entries(argTypes ?? {}).map(([name, arg]) => [name, summarizeArg(arg)]))
  return {
    described: Boolean(payload.description),
    props: props(payload.argTypes),
    subcomponents: Object.fromEntries(
      Object.entries(payload.subcomponents ?? {}).map(([name, sub]) => [
        name,
        sub.error ? { error: sub.error.message } : { props: props(sub.argTypes) },
      ]),
    ),
  }
}

function reasonBucket(message) {
  if (/props may be incomplete/.test(message)) return 'degraded'
  if (/found no component/.test(message)) return 'no component in session'
  if (/meta\.component/.test(message)) return 'meta.component unresolved'
  if (/no file for the import/.test(message)) return 'no file for import'
  return 'other'
}

// ── one variant in this process ────────────────────────────────────────────────────────────────────────────────────

async function runStories(variant, corpus, options) {
  const { root, src } = CORPORA[corpus]
  const rootDir = resolve(options.corpora, root)
  process.chdir(rootDir)
  const files = JSON.parse(readFileSync(options.files, 'utf8'))
  const reasons = {}
  const fallbacks = {}
  const warnings = []
  let currentFile = ''
  let provider
  let initMs = 0

  if (variant === 'baseline') {
    // React's react-component-meta needs the TypeScript 5/6 compiler API, which this package's own TypeScript 7 lacks.
    const typescript = pathToFileURL(options.typescript).href
    registerHooks({
      resolve: (specifier, context, next) =>
        specifier === 'typescript'
          ? { url: typescript, format: 'commonjs', shortCircuit: true }
          : next(specifier, context),
    })
    const react = dirname(require.resolve('@storybook/react/package.json'))
    const { createDocgenProvider } = await import(pathToFileURL(join(react, 'dist/docgen/docgen-worker.js')).href)
    provider = createDocgenProvider()
  } else {
    const { logger } = await import('storybook/internal/node-logger')
    const capture = (message) => {
      const text = String(message)
      if (text.includes(`${currentFile}:`) || text.startsWith('[caliper-docgen]')) {
        reasons[currentFile] = text
      }
    }
    logger.debug = capture
    logger.warn = (message) => {
      warnings.push(String(message))
      capture(message)
    }
    const { createDocgenProvider } = await import(pathToFileURL(join(options.dist, 'provider.js')).href)
    provider = createDocgenProvider({
      srcDirs: [src].flat(),
      tsconfigPath: options.tsconfig,
      ...VARIANTS[variant],
    })
  }

  const handed = Symbol('next')
  const wrapped = provider(async () => handed)
  const stories = {}
  const times = []
  const start = performance.now()
  for (const file of files) {
    const importPath = `./${file.slice(rootDir.length + 1)}`
    const title = importPath.replace(/^.*\//, '').replace(STORY_FILE, '')
    currentFile = importPath.slice(2)
    const entry = {
      type: 'story',
      subtype: 'story',
      id: `${title.toLowerCase()}--basic`,
      name: 'Basic',
      title,
      importPath,
      tags: [],
    }
    const t0 = performance.now()
    let payload
    let error
    try {
      payload = await wrapped({ entry })
    } catch (caught) {
      error = String(caught?.message ?? caught)
    }
    times.push(performance.now() - t0)
    if (error) stories[importPath] = { status: 'error', reason: error }
    else if (payload === handed || payload === undefined) {
      stories[importPath] = { status: 'fallback', reason: reasons[currentFile] ?? 'unknown' }
      const bucket = reasonBucket(reasons[currentFile] ?? '')
      fallbacks[bucket] = (fallbacks[bucket] ?? 0) + 1
    } else stories[importPath] = { status: 'answered', ...summarize(payload) }
    if (times.length === 1) initMs = times[0]
  }
  const totalMs = performance.now() - start
  times.sort((a, b) => a - b)
  const counts = { answered: 0, fallback: 0, error: 0 }
  for (const story of Object.values(stories)) counts[story.status]++
  return {
    variant,
    corpus,
    files: files.length,
    ...counts,
    fallbackReasons: fallbacks,
    // Which story each hand-off was and why, so a bucket can be read, not only counted.
    fallbackStories: Object.fromEntries(
      Object.entries(stories)
        .filter(([, story]) => story.status === 'fallback' || story.status === 'error')
        .map(([file, story]) => [file, story.reason]),
    ),
    totalMs: Math.round(totalMs),
    firstRequestMs: Math.round(initMs),
    medianMs: Math.round(times[Math.floor(times.length / 2)] ?? 0),
    maxRssMb: peakRssMb(),
    stories,
  }
}

/** The session alone: how many components would the addon hand on, and why. No story files needed. */
async function runSession(variant, corpus, options) {
  const { root, src } = CORPORA[corpus]
  const rootDir = resolve(options.corpora, root)
  process.chdir(rootDir)
  const { DocgenSession } = await import(pathToFileURL(join(options.dist, 'session.js')).href)
  const napi = await import('caliper-docgen-napi')
  const session = new DocgenSession(
    {
      srcDirs: [src].flat().map((dir) => resolve(rootDir, dir)),
      tsconfigPath: options.tsconfig,
      ...VARIANTS[variant],
    },
    napi.default ?? napi,
  )
  const start = performance.now()
  await session.fresh()
  const initMs = performance.now() - start
  const keys = [...session.components.keys()]
  const reasons = {}
  let degraded = 0
  let props = 0
  const docStart = performance.now()
  for (const key of keys) {
    const { doc, degradedBy } = await session.documentation(key)
    props += Object.keys(doc?.props ?? {}).length
    if (degradedBy.length === 0) continue
    degraded++
    for (const message of new Set(degradedBy.map(shape))) reasons[message] = (reasons[message] ?? 0) + 1
  }
  const topReasons = Object.entries(reasons)
    .toSorted((a, b) => b[1] - a[1])
    .slice(0, 8)
  return {
    variant,
    corpus,
    components: keys.length,
    list: [...session.components.values()].map((ref) => ({ name: ref.displayName, file: ref.filePath })),
    degraded,
    degradedPct: keys.length ? Math.round((degraded / keys.length) * 1000) / 10 : 0,
    topReasons,
    props,
    initMs: Math.round(initMs),
    docgenMs: Math.round(performance.now() - docStart),
    maxRssMb: peakRssMb(),
  }
}

// ── orchestration and comparison ───────────────────────────────────────────────────────────────────────────────────

function child(mode, variant, corpus, options) {
  const args = [
    fileURLToPath(import.meta.url),
    '--child',
    mode,
    '--variant',
    variant,
    '--corpus',
    corpus,
    '--corpora',
    options.corpora,
  ]
  if (options.files) args.push('--files', options.files)
  args.push('--dist', options.dist)
  args.push('--typescript', options.typescript)
  if (options.tsconfig) args.push('--tsconfig', options.tsconfig)
  const result = spawnSync(process.execPath, args, { encoding: 'utf8', maxBuffer: 1 << 30, cwd: here })
  if (result.status !== 0) return { variant, corpus, failed: (result.stderr || result.stdout).slice(-600) }
  if (!result.stdout.includes('@@RESULT@@')) return { variant, corpus, failed: 'no result' }
  return JSON.parse(result.stdout.slice(result.stdout.lastIndexOf('\n@@RESULT@@\n') + 12))
}

/**
 * Whether the lazy and the eager session answered every story alike: the same status, argTypes and fallback reason.
 * Returns the stories that differ per variant against `caliper-eager`; absent unless that variant ran.
 */
function sameAnswers(runs) {
  const eager = runs.find((r) => r.variant === 'caliper-eager' && !r.failed)
  if (!eager) return undefined
  const differing = {}
  for (const run of runs.filter((r) => r.variant !== 'caliper-eager' && r.variant.startsWith('caliper') && !r.failed)) {
    differing[run.variant] = Object.keys(eager.stories).filter(
      (file) => JSON.stringify(run.stories[file]) !== JSON.stringify(eager.stories[file]),
    )
  }
  return differing
}

/** Real story files that name a component. */
function realStories(rootDir, limit) {
  return walk(rootDir)
    .filter((file) => /^\s*component:/m.test(readFileSync(file, 'utf8')))
    .toSorted()
    .slice(0, limit)
}

const escapeRegExp = (text) => text.replaceAll(/[.*+?^${}()|[\]\\]/g, '\\$&')

/**
 * One generated story file per sampled component, beside the component so the nearest tsconfig is the one its own
 * stories would use: `export default { component }`. Components that are not a named or default export are skipped.
 */
function syntheticStories(list, sample) {
  const sorted = list.toSorted((a, b) => (a.file + a.name).localeCompare(b.file + b.name))
  const step = Math.max(1, Math.ceil(sorted.length / sample))
  const created = []
  for (const [index, { name, file }] of sorted.entries()) {
    if (index % step !== 0 || !/^[A-Z]\w*(\.[A-Z]\w*)?$/.test(name) || !/\.(tsx?)$/.test(file)) continue
    const source = readFileSync(file, 'utf8')
    const [root] = name.split('.')
    const escaped = escapeRegExp(root)
    const named =
      new RegExp(`export\\s+(?:declare\\s+)?(?:const|function|class)\\s+${escaped}\\b`).test(source) ||
      new RegExp(`export\\s*\\{[^}]*\\b${escaped}\\b[^}]*\\}`).test(source)
    const module = `./${basename(file).replace(/\.tsx?$/, '')}`
    const header = named
      ? `import { ${root} } from '${module}'`
      : /export\s+default\s/.test(source)
        ? `import ${root} from '${module}'`
        : undefined
    if (!header) continue
    const path = join(dirname(file), `__caliper_parity__${name.replace('.', '_')}.stories.tsx`)
    writeFileSync(
      path,
      `${header}\nexport default { title: 'Parity/${name}', component: ${name} }\nexport const Basic = {}\n`,
    )
    created.push(path)
  }
  return created
}

/** Compares the props both providers documented for one story, and counts what only the addon has. */
function compareProps(ours, theirs, into, story) {
  for (const [name, a] of Object.entries(ours)) {
    const b = theirs[name]
    if (!b) {
      into.extraProps++
      if (into.examples.length < 30 && into.extraProps % 37 === 0) into.examples.push(`${basename(story)}: ${name}`)
      if (a.described) into.extraDescribed++
      if (a.required) into.extraRequired++
      continue
    }
    into.sharedProps++
    const where = basename(story)
    if (a.kind !== b.kind) {
      into.kindDiffers++
      into.details.kind.push({ story: where, prop: name, ours: a.kind, theirs: b.kind, oursType: a.type, theirsType: b.type })
    }
    if (a.options && !b.options) into.optionsOnlyOurs++
    else if (!a.options && b.options) into.optionsOnlyTheirs++
    else if (a.options && b.options && a.options.toSorted().join('|') !== b.options.toSorted().join('|'))
      into.optionsDiffer++
    if (a.required !== b.required) into.requiredDiffers++
    if (a.default && !b.default) into.defaultOnlyOurs++
    else if (!a.default && b.default) {
      into.defaultOnlyTheirs++
      into.details.defaults.push({ story: where, prop: name, theirs: b.default })
    }
    else if (a.default && b.default && a.default.replaceAll(/['"]/g, '') !== b.default.replaceAll(/['"]/g, ''))
      into.defaultDiffers++
    if (a.described && !b.described) into.describedOnlyOurs++
    else if (!a.described && b.described) into.describedOnlyTheirs++
    if (a.type === b.type) into.sameTypeText++
  }
  for (const name of Object.keys(theirs)) {
    if (name in ours) continue
    into.missingProps++
    into.details.missing.push({ story: basename(story), prop: name, theirs: theirs[name].type })
  }
}

function diff(ours, baseline) {
  const into = {
    both: 0,
    onlyOurs: 0,
    onlyTheirs: 0,
    neither: 0,
    extraProps: 0,
    extraDescribed: 0,
    extraRequired: 0,
    missingProps: 0,
    sharedProps: 0,
    sameTypeText: 0,
    kindDiffers: 0,
    optionsOnlyOurs: 0,
    optionsOnlyTheirs: 0,
    optionsDiffer: 0,
    requiredDiffers: 0,
    defaultOnlyOurs: 0,
    defaultOnlyTheirs: 0,
    defaultDiffers: 0,
    describedOnlyOurs: 0,
    describedOnlyTheirs: 0,
    propsOnStoriesOnlyWeAnswer: 0,
    examples: [],
    storiesWithMissing: 0,
    // Every disagreement, by story and prop, in the JSON report.
    details: { kind: [], defaults: [], missing: [] },
  }
  for (const [path, a] of Object.entries(ours.stories)) {
    const b = baseline.stories[path]
    const weAnswer = a.status === 'answered'
    const theyAnswer = b?.status === 'answered' && Object.keys(b.props).length > 0
    if (weAnswer && theyAnswer) {
      into.both++
      const before = into.missingProps
      compareProps(a.props, b.props, into, path)
      if (into.missingProps > before) into.storiesWithMissing++
      for (const [name, sub] of Object.entries(a.subcomponents)) {
        const other = b.subcomponents[name]
        if (sub.props && other?.props) compareProps(sub.props, other.props, into, path)
      }
    } else if (weAnswer) {
      into.onlyOurs++
      into.propsOnStoriesOnlyWeAnswer += Object.keys(a.props).length
    } else if (theyAnswer) into.onlyTheirs++
    else into.neither++
  }
  return into
}

const pct = (n, d) => (d ? `${Math.round((n / d) * 1000) / 10}%` : 'n/a')

function table(rows) {
  const [head, ...body] = rows
  return [
    `| ${head.join(' | ')} |`,
    `| ${head.map(() => '---').join(' | ')} |`,
    ...body.map((r) => `| ${r.join(' | ')} |`),
  ].join('\n')
}

function markdown(report) {
  const lines = ['# Storybook parity', '', `Generated ${report.generated}.`, '']
  lines.push("## Fallback rate by session (components the addon would hand to Storybook's docgen)", '')
  lines.push(
    table([
      ['corpus', 'variant', 'components', 'degraded', 'degraded %', 'props', 'init ms', 'docgen ms', 'peak RSS MB'],
      ...report.sessions.map((s) =>
        s.failed
          ? [s.corpus, s.variant, 'failed', '', '', '', '', '', '']
          : [
              s.corpus,
              s.variant,
              s.components,
              s.degraded,
              `${s.degradedPct}%`,
              s.props,
              s.initMs,
              s.docgenMs,
              s.maxRssMb,
            ],
      ),
    ]),
    '',
  )
  for (const s of report.sessions.filter((x) => !x.failed && x.topReasons.length > 0 && x.variant === 'caliper')) {
    lines.push(`Top degradation reasons, ${s.corpus}: ${s.topReasons.map(([m, n]) => `${m} (${n})`).join('; ')}`, '')
  }
  for (const [corpus, entry] of Object.entries(report.stories)) {
    lines.push(`## Stories: ${corpus}`, '')
    lines.push(
      table([
        [
          'variant',
          'story files',
          'answered',
          'handed on',
          'errors',
          'handed-on reasons',
          'total s',
          'first ms',
          'median ms',
          'peak RSS MB',
        ],
        ...entry.runs.map((r) =>
          r.failed
            ? [r.variant, 'failed', '', '', '', r.failed.slice(0, 80), '', '', '', '']
            : [
                r.variant,
                r.files,
                r.answered,
                r.fallback,
                r.error,
                Object.entries(r.fallbackReasons)
                  .map(([k, v]) => `${k} ${v}`)
                  .join(', '),
                (r.totalMs / 1000).toFixed(1),
                r.firstRequestMs,
                r.medianMs,
                r.maxRssMb,
              ],
        ),
      ]),
      '',
    )
    for (const [variant, differing] of Object.entries(entry.sameAnswers ?? {})) {
      lines.push(
        differing.length === 0
          ? `\`${variant}\` answers every story as \`caliper-eager\` does.`
          : `\`${variant}\` differs from \`caliper-eager\` on ${differing.length} stories: ${differing.slice(0, 5).join(', ')}`,
        '',
      )
    }
    for (const [variant, d] of Object.entries(entry.diffs)) {
      lines.push(`### ${corpus}: ${variant} against Storybook's docgen`, '')
      lines.push(
        table([
          ['measure', 'count'],
          ['stories both document', d.both],
          ['stories only the addon documents', `${d.onlyOurs} (${d.propsOnStoriesOnlyWeAnswer} props)`],
          ['stories only Storybook documents', d.onlyTheirs],
          [
            '**extra props the addon lists**',
            `${d.extraProps} (${d.extraDescribed} described, ${d.extraRequired} required)`,
          ],
          ['props Storybook lists that the addon lacks', `${d.missingProps} in ${d.storiesWithMissing} stories`],
          ['shared props', d.sharedProps],
          ['same type text', pct(d.sameTypeText, d.sharedProps)],
          ['different control kind', d.kindDiffers],
          [
            'options: only addon / only Storybook / differ',
            `${d.optionsOnlyOurs} / ${d.optionsOnlyTheirs} / ${d.optionsDiffer}`,
          ],
          ['required differs', d.requiredDiffers],
          [
            'default: only addon / only Storybook / differ',
            `${d.defaultOnlyOurs} / ${d.defaultOnlyTheirs} / ${d.defaultDiffers}`,
          ],
          ['description: only addon / only Storybook', `${d.describedOnlyOurs} / ${d.describedOnlyTheirs}`],
        ]),
        '',
      )
      if (d.examples.length > 0) lines.push(`Sample of extra props: ${d.examples.join(', ')}`, '')
    }
  }
  return lines.join('\n')
}

async function main() {
  const { values } = parseArgs({
    options: {
      corpora: { type: 'string' },
      only: { type: 'string' },
      out: { type: 'string', default: 'storybook-parity' },
      limit: { type: 'string' },
      sample: { type: 'string' },
      files: { type: 'string' },
      dist: { type: 'string' },
      typescript: { type: 'string' },
      tsconfig: { type: 'string' },
      child: { type: 'string' },
      variant: { type: 'string' },
      corpus: { type: 'string' },
      'no-baseline': { type: 'boolean', default: false },
      // Which story variants to run (default: all), and whether to run only the one session that lists components.
      variants: { type: 'string' },
      'one-session': { type: 'boolean', default: false },
    },
  })
  if (!values.corpora) throw new Error('--corpora <dir holding the cloned repositories> is required')
  const options = {
    corpora: resolve(values.corpora),
    limit: values.limit ? Number(values.limit) : Infinity,
    sample: values.sample ? Number(values.sample) : 100,
    files: values.files,
    // The addon build to measure; another build of the package runs the same stories for a before/after.
    dist: resolve(values.dist ?? join(here, '../dist')),
    typescript: resolve(
      values.typescript ?? join(here, '../../../apps/validate/node_modules/typescript/lib/typescript.js'),
    ),
    tsconfig: values.tsconfig,
  }

  if (values.child) {
    const result =
      values.child === 'session'
        ? await runSession(values.variant, values.corpus, options)
        : await runStories(values.variant, values.corpus, options)
    process.stdout.write(`\n@@RESULT@@\n${JSON.stringify(result)}`)
    return
  }

  const names = (values.only ? values.only.split(',') : Object.keys(CORPORA)).filter((name) =>
    existsSync(join(options.corpora, CORPORA[name].root)),
  )
  const report = { generated: new Date().toISOString().slice(0, 10), sessions: [], stories: {} }
  for (const corpus of names) {
    for (const variant of values['one-session'] ? ['caliper'] : SESSION_VARIANTS) {
      console.error(`session ${corpus} ${variant}`)
      report.sessions.push(child('session', variant, corpus, options))
    }
    const rootDir = join(options.corpora, CORPORA[corpus].root)
    const list = report.sessions.find((x) => x.corpus === corpus && x.variant === 'caliper')?.list ?? []
    const real = realStories(rootDir, options.limit)
    const generated = []
    const sets = real.length > 0 ? [[`${corpus}: real stories (${real.length})`, real]] : []
    try {
      const synthetic = syntheticStories(list, options.sample)
      generated.push(...synthetic)
      if (synthetic.length > 0) sets.push([`${corpus}: sampled components (${synthetic.length})`, synthetic])
      for (const [title, files] of sets) {
        const dir = mkdtempSync(join(tmpdir(), 'caliper-parity-'))
        const listing = join(dir, 'files.json')
        writeFileSync(listing, JSON.stringify(files))
        const runs = []
        const chosen = values.variants ? values.variants.split(',') : Object.keys(VARIANTS)
        const variants = values['no-baseline'] ? chosen : ['baseline', ...chosen]
        for (const variant of variants) {
          console.error(`stories ${title} ${variant}`)
          runs.push(child('stories', variant, corpus, { ...options, files: listing }))
        }
        rmSync(dir, { recursive: true, force: true })
        const baseline = runs.find((r) => r.variant === 'baseline' && !r.failed)
        const diffs = {}
        if (baseline) {
          for (const run of runs.filter((r) => r.variant !== 'baseline' && !r.failed))
            diffs[run.variant] = diff(run, baseline)
        }
        report.stories[title] = {
          runs: runs.map(({ stories, ...rest }) => rest),
          diffs,
          sameAnswers: sameAnswers(runs),
        }
      }
    } finally {
      for (const path of generated) rmSync(path, { force: true })
    }
  }
  for (const session of report.sessions) delete session.list
  mkdirSync(dirname(resolve(values.out)), { recursive: true })
  writeFileSync(`${values.out}.json`, JSON.stringify(report, null, 2))
  writeFileSync(`${values.out}.md`, markdown(report))
  console.log(markdown(report))
}

await main()
