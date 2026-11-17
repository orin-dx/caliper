// The universe of src/oracle.ts: every own prop the checker finds on an exported component, with what the checker says
// of it and what each tool reports (Caliper, react-docgen-typescript and react-docgen). The checker and the corpus
// root come in as arguments, so this file reads only the dumps the tools wrote. Scoring is src/tool-score.ts.
import { readFileSync, realpathSync } from 'node:fs'
import { relative } from 'node:path'
import { layerMembers } from './canonical.ts'
import { caliperNorm } from './caliper-norm.ts'
import { SHAPE_REASONS, type ShapeReason } from './component-shape.ts'
import type { Norm } from './oracle-compare.ts'
import { isDependency } from './path-class.ts'
import { VERDICTS, type Verdict } from './vocabulary.ts'
import { rdgNorm, rdtNorm } from './tool-norm.ts'
import {
  scoreReported,
  scoreTool,
  type Reported,
  type Stratum,
  type ToolScore,
  type UniverseProp,
} from './tool-score.ts'

export const pathKey = (path: string) =>
  process.platform === 'darwin' || process.platform === 'win32' ? path.toLowerCase() : path

/** The files of a component's collapsed inheritance layers: Caliper lists their props as members, not as props. A layer
 * is collapsed when it lists members. Its size does not tell: `Omit` can leave one at 30 props or fewer, and with
 * `--collapse-inherited-props none` a layer of more than 30 is expanded. The output omits an empty `members`, so a layer
 * without it names nothing and has nothing to attribute. */
export const collapsedLayerFiles = (caliper: any, component: any): Set<string> =>
  new Set(
    (component?.inheritance ?? [])
      .filter((layer: any) => layerMembers(caliper, layer).length > 0)
      .map((layer: any) => pathKey(String(layer.fileName))),
  )

/** A component as the tools are matched to the checker's: the file and the name the tool lists it under
 * (`listedName` in src/checker-components.ts). The tools have no exports to match on, so this is the one join by name. */
const componentKey = (file: string, name: string) => `${file}\0${name}`

export interface ToolDump {
  /** Props by component; a file with two components of one name merges them, the first prop winning. */
  props: Map<string, Record<string, any>>
  /** Files the tool threw on, as paths relative to the corpus root. */
  failed: Set<string>
}

/** A dump `scripts/rdt.mjs` or `scripts/rdg.mjs` wrote. `rel` makes a path relative to the corpus root. */
export function loadTool(path: string | undefined, rel: (path: string) => string): ToolDump | undefined {
  if (!path) return undefined
  // A path that no longer exists is kept as written.
  const relOrAsIs = (p: string) => {
    try {
      return rel(p)
    } catch {
      return p
    }
  }
  const dump = JSON.parse(readFileSync(path, 'utf8'))
  const props = new Map<string, Record<string, any>>()
  for (const c of dump.components) {
    const key = componentKey(relOrAsIs(c.file), c.name ?? '')
    props.set(key, { ...c.props, ...props.get(key) })
  }
  return { props, failed: new Set<string>((dump.failedFiles ?? []).map((f: string) => relOrAsIs(f))) }
}

/** What a tool reports for one prop of one component: its type as a `Norm`, or why it has none. */
function reported(tool: ToolDump, at: { key: string; prop: string }, norm: (p: any) => Norm): Reported {
  if (tool.failed.has(at.key.split('\0')[0])) return 'failed_file'
  const props = tool.props.get(at.key)
  if (!props) return 'component'
  return at.prop in props ? norm(props[at.prop]) : 'prop'
}

type Entry = UniverseProp & { caliper: Reported; rdt?: Reported; rdg?: Reported; nonComponent?: ShapeReason }
interface Match {
  key: string
  component: any
}

/** Where the checker declares a prop, by its first declaration: the component's own file, another file of the corpus, or
 * a dependency. */
function stratumOf(p: any, file: string): Stratum {
  const path = p.declarations?.[0]?.path
  if (!path) return 'unknown'
  if (isDependency(String(path))) return 'dependency'
  return pathKey(String(path)) === pathKey(realpathSync(file)) ? 'same_file' : 'other_project_file'
}

/** What Caliper reports for one prop: the type, or why it has none. An ambiguous name is not scored for Caliper, as in
 * the verdicts of src/oracle.ts. */
function caliperReported(match: Match | undefined, p: any, caliper: any): Reported {
  const id = match?.component.props[p.name]
  if (id !== undefined) return caliperNorm(caliper.props[id].type)
  if (!match) return 'component'
  const layerFiles = collapsedLayerFiles(caliper, match.component)
  return p.declarations?.some((d: any) => layerFiles.has(pathKey(String(d.path)))) ? 'collapsed' : 'prop'
}

interface Environment {
  root: string
  caliper: any
  /** The checker and its normaliser (src/oracle-checker.ts). */
  checker: any
  checkerNorm: (type: any) => Norm
  rdt?: ToolDump
  rdg?: ToolDump
}

/** What the universe left out as no component: the functions and props, in all and by the rule that decided it. */
export interface Excluded {
  components: number
  props: number
  by_reason: Partial<Record<ShapeReason, { components: number; props: number }>>
}

type Scores = Record<string, ToolScore & { failed_files?: number }>

/** Collects the universe as components are visited. */
export function universeRecorder(env: Environment) {
  const entries: Entry[] = []
  const rel = (path: string) => relative(env.root, realpathSync(path))
  const scoreOver = (over: readonly Entry[]): Scores => ({
    caliper: scoreTool(over, (u) => u.caliper),
    // Caliper over the components it reports only (src/tool-score.ts `scoreReported`): the weekly headline.
    caliper_reported: scoreReported(over, (u) => u.caliper),
    ...(env.rdt && { rdt: scoreTool(over, (u) => u.rdt!) }),
    ...(env.rdg && { react_docgen: { ...scoreTool(over, (u) => u.rdg!), failed_files: env.rdg.failed.size } }),
  })
  return {
    entries,
    /** `nonComponent` is why the shape rules (src/component-shape.ts) say the function is no component. A function
     * Caliper reports as one stays in the universe whatever its shape: the rules say what Caliper need not list, not what
     * it must not. */
    record({
      own,
      name,
      file,
      matches,
      nonComponent,
    }: {
      own: any[]
      name: string
      file: string
      matches: Match[]
      nonComponent?: ShapeReason
    }) {
      if (own.length === 0) return
      const key = componentKey(rel(file), name)
      const types = env.checker.getTypeOfSymbol(own)
      const [match] = matches.length === 1 ? matches : []
      const excluded = matches.length === 0 ? nonComponent : undefined
      for (const [i, p] of own.entries()) {
        entries.push({
          component: key,
          prop: p.name,
          stratum: stratumOf(p, file),
          checker: env.checkerNorm(types[i]),
          caliper: caliperReported(match, p, env.caliper),
          rdt: env.rdt && reported(env.rdt, { key, prop: p.name }, (t) => rdtNorm(t.typeObject)),
          rdg: env.rdg && reported(env.rdg, { key, prop: p.name }, (t) => rdgNorm(t.tsType)),
          ...(excluded && { nonComponent: excluded }),
        })
      }
    },
    /** One score per tool over the candidate components: the universe without the functions the shape rules exclude.
     * Caliper is scored over the universe as well as over the props both list (`counts`, the verdicts of src/oracle.ts):
     * the two must agree, or the universe is not the one the verdicts were made over. */
    score(counts: Record<Verdict, number>): Scores {
      const scores = scoreOver(entries.filter((e) => !e.nonComponent))
      for (const verdict of VERDICTS) {
        if (scores.caliper.verdicts[verdict] !== counts[verdict]) {
          console.error(
            `warning: Caliper "${verdict}" is ${counts[verdict]} over the shared props, not ${scores.caliper.verdicts[verdict]} over the universe`,
          )
        }
      }
      return scores
    },
    /** The same scores over every function the checker takes for a component: the universe before the shape rules. */
    scoreAll(): Scores {
      return scoreOver(entries)
    },
    /** What the shape rules left out. */
    excluded(): Excluded {
      const components = new Set<string>()
      const byReason = new Map<ShapeReason, { components: Set<string>; props: number }>()
      let props = 0
      for (const e of entries) {
        if (!e.nonComponent) continue
        components.add(e.component)
        props++
        const reason = byReason.get(e.nonComponent) ?? { components: new Set<string>(), props: 0 }
        reason.components.add(e.component)
        reason.props++
        byReason.set(e.nonComponent, reason)
      }
      const by_reason: Excluded['by_reason'] = {}
      for (const reason of SHAPE_REASONS) {
        const found = byReason.get(reason)
        if (found) by_reason[reason] = { components: found.components.size, props: found.props }
      }
      return { components: components.size, props, by_reason }
    },
  }
}
