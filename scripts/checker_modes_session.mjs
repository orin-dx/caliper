#!/usr/bin/env node
// Drives one lazy session the way the Storybook addon does (createSession, initializeSessionIndex, then one
// sessionComponentAnswers call per component) and prints one JSON document: the time of each step, each answer's props,
// and whether the TypeScript checker answered or the structural result stood. scripts/checker_modes.py runs it once
// per mode and corpus and passes the session options (it builds them to match the addon's, and a test keeps them so).
//
// node scripts/checker_modes_session.mjs --options JSON [--count N] [--seed S] [--second-pass-wait MS]
//
// Nothing runs between two requests of the first pass except storing the parsed answer: building the records and any
// measurement of the checker's memory happen after the loop or outside this process, so that the checker, which keeps
// working in the background after a request that timed out, gets no idle time that the timings do not count.
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { performance } from 'node:perf_hooks'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const napi = createRequire(import.meta.url)(join(here, '..', 'packages', 'napi'))

const argv = process.argv.slice(2)
const arg = (name, fallback) => {
  const at = argv.indexOf(name)
  return at >= 0 ? argv[at + 1] : fallback
}
const optionsText = arg('--options')
if (!optionsText) throw new Error('--options is required')
const options = JSON.parse(optionsText)
const count = Number(arg('--count', '50'))
const seed = Number(arg('--seed', '1'))
const secondPassWait = Number(arg('--second-pass-wait', '2000'))
const semantic = options.semanticTypes ?? 'off'

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
  const has = (pattern) => messages.some((m) => pattern.test(m))
  let outcome = 'none'
  if (has(/has not answered yet/)) outcome = 'timed_out'
  else if (has(/backend unavailable/)) outcome = 'unavailable'
  else if (has(/kept their structural result/)) outcome = 'fallback'
  else if (has(/resolved by the TypeScript checker/)) outcome = 'checker'
  return { messages, outcome }
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

/** One request, parsed as the addon parses it; the time is this call and that parse and nothing else. */
function ask(key) {
  const t = performance.now()
  const answer = JSON.parse(napi.sessionComponentAnswers(sessionId, [key]))[key]
  return { key, ms: performance.now() - t, answer }
}

const asked = []
for (const key of keys) asked.push(ask(key))

// The checker's late answer serves the next request of the same component while no file changed: ask again. The wait
// and this pass are for that question only; the parent does not time this script as a whole for an on-demand run.
const askedAgain = []
if (semantic === 'on-demand') {
  await sleep(secondPassWait)
  for (const key of keys) askedAgain.push(ask(key))
}

napi.closeSession(sessionId)

let cumulative = 0
const record = ({ key, ms, answer }) => ({
  key,
  ms: Number(ms.toFixed(2)),
  present: Boolean(answer),
  ...placed(key),
  props: answer ? propsOf(answer.docgenInfo) : null,
  backend: answer ? backendOf(answer.diagnostics) : { messages: [], outcome: 'none' },
})
const results = asked.map((one) => {
  cumulative += one.ms
  return Object.assign(record(one), { cumulative_ms: Number(cumulative.toFixed(2)) })
})

const report = {
  options: { ...options, srcDirs: undefined, cacheDir: undefined },
  create_ms: Number(createMs.toFixed(2)),
  index_ms: Number(indexMs.toFixed(2)),
  components_listed: allKeys.length,
  asked: keys.length,
  results,
  second_pass: askedAgain.map((one) => record(one)),
}
process.stdout.write(JSON.stringify(report))
process.exit(0)
