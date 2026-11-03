#!/usr/bin/env node
// Drives one lazy session the way the Storybook addon does (createSession, initializeSessionIndex, then one
// sessionComponentAnswers call per component) and prints one JSON document: the time of each step, each answer's props,
// and whether the TypeScript checker answered or the structural result stood. scripts/checker_modes.py runs it once
// per mode and corpus.
//
// node scripts/checker_modes_session.mjs --src DIR [--count N] [--seed S] [--semantic off|on-demand] [--wait MS]
//   [--tsc PATH] [--second-pass-wait MS]
import { execFile } from 'node:child_process'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { performance } from 'node:perf_hooks'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

const here = dirname(fileURLToPath(import.meta.url))
const napi = createRequire(import.meta.url)(join(here, '..', 'packages', 'napi'))

const argv = process.argv.slice(2)
const arg = (name, fallback) => {
  const at = argv.indexOf(name)
  return at >= 0 ? argv[at + 1] : fallback
}
const src = arg('--src')
if (!src) throw new Error('--src is required')
const count = Number(arg('--count', '50'))
const seed = Number(arg('--seed', '1'))
const semantic = arg('--semantic', 'off')
const wait = arg('--wait')
const tsc = arg('--tsc')
const secondPassWait = Number(arg('--second-pass-wait', '2000'))

const options = {
  srcDirs: [src],
  lazy: true,
  htmlAttributes: 'full',
  cache: false,
  loadDependencies: true,
  semanticTypes: semantic,
  ...(wait === undefined ? {} : { semanticWaitMs: Number(wait) }),
  ...(tsc === undefined ? {} : { tscPath: tsc }),
}

/** A seeded shuffle, so every mode asks the same components in the same order. */
function shuffled(keys, seedValue) {
  let state = (seedValue * 2654435761) >>> 0 || 1
  const next = () => {
    state ^= state << 13
    state >>>= 0
    state ^= state >>> 17
    state ^= state << 5
    state >>>= 0
    return state / 2 ** 32
  }
  const out = [...keys]
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(next() * (i + 1))
    ;[out[i], out[j]] = [out[j], out[i]]
  }
  return out
}

/** The props of one answer as flat facts: the type text, whether it is required, and the default. */
function propsOf(doc) {
  const props = {}
  for (const [name, prop] of Object.entries(doc?.props ?? {})) {
    props[name] = {
      type: prop.type?.raw ?? prop.type?.name ?? null,
      required: Boolean(prop.required),
      default: prop.defaultValue?.value ?? null,
      react_declared: (prop.parent?.fileName ?? '').includes('/node_modules/@types/react/'),
    }
  }
  return props
}

/** What the checker did for one answer, from the `SEMANTIC_BACKEND` diagnostics it carries. */
function backendOf(diagnostics) {
  const messages = (diagnostics ?? []).filter((d) => d.code === 'SEMANTIC_BACKEND').map((d) => d.message)
  const waited = messages.some((m) => /has not answered yet/.test(m))
  const kept = messages.some((m) => /kept their structural result/.test(m))
  const resolved = messages.some((m) => /resolved by the TypeScript checker/.test(m))
  const unavailable = messages.some((m) => /backend unavailable/.test(m))
  return {
    messages,
    outcome: waited ? 'timed_out' : unavailable ? 'unavailable' : kept ? 'fallback' : resolved ? 'checker' : 'none',
  }
}

const run = promisify(execFile)

/** Resident memory of the child processes of this one (the `tsc` the checker spawned), in MiB. */
async function childRssMiB() {
  try {
    const { stdout: children } = await run('pgrep', ['-P', String(process.pid)])
    const pids = children.trim().split('\n').filter(Boolean)
    if (pids.length === 0) return 0
    const { stdout } = await run('ps', ['-o', 'rss=', '-p', pids.join(',')])
    const sizes = stdout
      .trim()
      .split('\n')
      .map((line) => Number(line.trim()) / 1024)
    return Math.max(...sizes)
  } catch {
    return 0
  }
}

const startedAt = performance.now()
const sessionId = napi.createSession(options)
const createMs = performance.now() - startedAt
const indexStart = performance.now()
const index = JSON.parse(await napi.initializeSessionIndex(sessionId, options))
const indexMs = performance.now() - indexStart
/** Where a component is declared and what it is called, from the index: an answer carries neither. */
const placed = (key) => ({
  file: index.components?.[key]?.filePath ?? null,
  display_name: index.components?.[key]?.displayName ?? null,
})

const allKeys = napi.sessionComponentKeys(sessionId).toSorted()
const keys = count > 0 ? shuffled(allKeys, seed).slice(0, count) : allKeys

const results = []
let cumulative = 0
let tscRssMax = 0
for (const key of keys) {
  const t = performance.now()
  const answers = JSON.parse(napi.sessionComponentAnswers(sessionId, [key]))
  const ms = performance.now() - t
  cumulative += ms
  const answer = answers[key]
  results.push({
    key,
    ms: Number(ms.toFixed(2)),
    cumulative_ms: Number(cumulative.toFixed(2)),
    present: Boolean(answer),
    ...placed(key),
    props: answer ? propsOf(answer.docgenInfo) : null,
    backend: answer ? backendOf(answer.diagnostics) : { messages: [], outcome: 'none' },
  })
  tscRssMax = Math.max(tscRssMax, await childRssMiB())
}

// The checker's late answer serves the next request of the same component while no file changed: ask again.
let secondPass = []
if (semantic === 'on-demand') {
  await sleep(secondPassWait)
  secondPass = keys.map((key) => {
    const t = performance.now()
    const answers = JSON.parse(napi.sessionComponentAnswers(sessionId, [key]))
    const answer = answers[key]
    return {
      key,
      ms: Number((performance.now() - t).toFixed(2)),
      ...placed(key),
      props: answer ? propsOf(answer.docgenInfo) : null,
      backend: answer ? backendOf(answer.diagnostics) : { messages: [], outcome: 'none' },
    }
  })
  tscRssMax = Math.max(tscRssMax, await childRssMiB())
}

napi.closeSession(sessionId)

const report = {
  options: { ...options, srcDirs: undefined },
  create_ms: Number(createMs.toFixed(2)),
  index_ms: Number(indexMs.toFixed(2)),
  components_listed: allKeys.length,
  asked: keys.length,
  tsc_rss_mib: Math.round(tscRssMax),
  results,
  second_pass: secondPass,
}
process.stdout.write(JSON.stringify(report))
process.exit(0)
