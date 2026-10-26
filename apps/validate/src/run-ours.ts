import { execSync } from 'node:child_process'
import { existsSync, readdirSync, statSync } from 'node:fs'
import { resolve } from 'node:path'
import type { ToolResult, NormalizedOutput } from './types.ts'

const __dirname = import.meta.dirname
const FIXTURES_ROOT = resolve(__dirname, '../../../fixtures')
// Prefer release build (faster, has all fixes); fall back to debug for dev convenience.
const CLI = existsSync(resolve(__dirname, '../../../target/release/caliper-docgen'))
  ? resolve(__dirname, '../../../target/release/caliper-docgen')
  : resolve(__dirname, '../../../target/debug/caliper-docgen')

function discoverLibraries(): string[] {
  return readdirSync(FIXTURES_ROOT)
    .filter((d) => statSync(resolve(FIXTURES_ROOT, d)).isDirectory())
    .toSorted()
}

function propTypeToString(pt: any): string {
  if (!pt || typeof pt !== 'object') return 'unknown'
  switch (pt.kind) {
    case 'string': {
      return 'string'
    }
    case 'number': {
      return 'number'
    }
    case 'boolean': {
      return 'boolean'
    }
    case 'null': {
      return 'null'
    }
    case 'undefined': {
      return 'undefined'
    }
    case 'any': {
      return 'any'
    }
    case 'never': {
      return 'never'
    }
    case 'unknown': {
      return 'unknown'
    }
    case 'void': {
      return 'void'
    }
    case 'reactNode': {
      return 'ReactNode'
    }
    case 'cssProperties': {
      return 'CSSProperties'
    }
    case 'elementType': {
      return 'ElementType'
    }
    case 'sxProps': {
      return 'SxProps'
    }
    case 'stringLiteral': {
      return JSON.stringify(pt.value)
    }
    case 'numberLiteral': {
      return String(pt.value)
    }
    case 'boolLiteral': {
      return String(pt.value)
    }
    case 'union': {
      return (pt.members as any[]).map((member) => propTypeToString(member)).join(' | ')
    }
    case 'intersection': {
      return (pt.members as any[]).map((member) => propTypeToString(member)).join(' & ')
    }
    case 'array': {
      return `${propTypeToString(pt.element)}[]`
    }
    case 'tuple': {
      return `[${(pt.elements as any[]).map((member) => propTypeToString(member)).join(', ')}]`
    }
    case 'object': {
      return '{ ... }'
    }
    case 'named': {
      return pt.args?.length ? `${pt.name}<${pt.args.map((member) => propTypeToString(member)).join(', ')}>` : pt.name
    }
    case 'function': {
      const typeParams = pt.typeParams?.length ? `<${pt.typeParams.join(', ')}>` : ''
      const params = pt.params
        .map((param: any, index: number) => `${param.name ?? `__${index}`}: ${propTypeToString(param.type)}`)
        .join(', ')
      return `${typeParams}(${params}) => ${propTypeToString(pt.returnType)}`
    }
    case 'ref': {
      return pt.element ? `Ref<${pt.element}>` : 'Ref'
    }
    case 'htmlAttributes': {
      return `${pt.element}HTMLAttributes`
    }
    case 'literalUnion': {
      return pt.members.map((m: string) => JSON.stringify(m)).join(' | ')
    }
    case 'opaque': {
      return pt.raw
    }
    default: {
      return JSON.stringify(pt)
    }
  }
}

function normalize(output: any): NormalizedOutput {
  const result: NormalizedOutput = {}
  for (const [_key, comp] of Object.entries(output.components ?? {})) {
    const c = comp as any
    result[c.displayName] = {
      displayName: c.displayName,
      description: c.description ?? '',
      props: Object.fromEntries(
        Object.entries(c.props ?? {}).map(([propName, id]: [string, any]) => {
          const prop = output.props[id]
          return [
            propName,
            {
              name: propName,
              required: prop.required ?? false,
              type: propTypeToString(prop.type),
              description: prop.description ?? '',
              defaultValue: prop.defaultValue?.value,
            },
          ]
        }),
      ),
    }
  }
  return result
}

// RDT always fully expands the real HTMLAttributes/AriaAttributes/<Element>HTMLAttributes
// interface chain (~250-300 attrs) — it has a real type checker to walk that chain. Our
// default (`curated`) intentionally surfaces a hand-picked ~15-20 common attrs instead (see
// ARCHITECTURE.md). Comparing curated-mode output against RDT's always-full output would
// manufacture a huge "missing props" number that's actually just a default-mode mismatch, not
// a real coverage gap — pass `--html-attributes full` here so this baseline is apples-to-apples
// with what RDT itself always does.
const HTML_ATTRIBUTES_MODE = process.env.HTML_ATTRIBUTES_MODE ?? 'full'
// The fixtures are single copied files with no installed packages, so the harness pins dependency loading off
// (it is on by default, ADR 0019). `CALIPER_LOAD_DEPENDENCIES=1` runs with `--load-dependencies` (ADR 0010), which
// RDT's checker always does in effect.
const LOAD_DEPENDENCIES =
  process.env.CALIPER_LOAD_DEPENDENCIES === '1' ? ' --load-dependencies' : ' --no-load-dependencies'

const libraries = discoverLibraries()
const results: ToolResult[] = []

// One CLI invocation across every fixture library at once (`--src` takes a
// comma-delimited list — see crates/cli/src/main.rs's `value_delimiter = ','`)
// instead of one process per library. Spawning 21 separate processes measured
// ~885ms dominated by process-startup overhead (~40ms/spawn), even though the
// tool's own internal "durationMs" stat for the combined single-invocation
// run is ~40-50ms — a single invocation is both the fair cold-extraction
// number AND how anyone would actually run this CLI in practice; nobody
// invokes it once per library directory in a real build.
{
  const srcDirs = libraries.map((lib) => resolve(FIXTURES_ROOT, lib)).join(',')
  const start = performance.now()
  try {
    const raw = execSync(
      `${CLI} extract --src ${srcDirs} --format canonical --html-attributes ${HTML_ATTRIBUTES_MODE}${LOAD_DEPENDENCIES}`,
      {
        encoding: 'utf8',
        stdio: ['pipe', 'pipe', 'pipe'],
        maxBuffer: 100 * 1024 * 1024,
      },
    )
    // CLI prints a spinner line then JSON — find the JSON start
    const jsonStart = raw.indexOf('{')
    const json = jsonStart === -1 ? raw : raw.slice(jsonStart)
    const parsed = JSON.parse(json)
    const durationMs = performance.now() - start

    for (const [_key, comp] of Object.entries(parsed.components ?? {})) {
      const c = comp as any
      const inheritedElements = (c.inheritance ?? []).map((l: any) => l.htmlElement).filter(Boolean)
      const notableInheritedNames = Object.keys(c.notableInherited ?? {})
      // Derive the owning library dir from the absolute filePath (the fixtures/<lib>/
      // segment) now that one invocation covers every library at once.
      const fixturesIdx = c.filePath.indexOf('/fixtures/')
      const relPath = fixturesIdx === -1 ? c.filePath : c.filePath.slice(fixturesIdx + '/fixtures/'.length)
      const [lib] = relPath.split('/')
      const basename =
        c.filePath
          .split('/')
          .pop()
          ?.replace(/\.tsx?$/, '')
          .replace(/\.d$/, '') ?? c.displayName
      results.push({
        tool: 'caliper-docgen',
        fixture: `${lib}/${basename}/${c.displayName}`,
        durationMs,
        // Components refer to their props by id in the output's shared `props` table (ADR 0009).
        output: normalize({ components: { [c.displayName]: c }, props: parsed.props }),
        inheritedElements,
        notableInheritedNames,
      })
    }
  } catch (error: any) {
    results.push({
      tool: 'caliper-docgen',
      fixture: 'all-fixtures',
      durationMs: performance.now() - start,
      output: {},
      error: error.stderr ?? error.message,
    })
  }
}

console.log(JSON.stringify(results, null, 2))
