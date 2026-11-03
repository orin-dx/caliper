// The universe of src/oracle.ts: every own prop the checker finds on an exported component, with what the checker says
// of it and what each tool reports (Caliper, react-docgen-typescript and react-docgen). The checker and the corpus
// root come in as arguments, so this file reads only the dumps the tools wrote. Scoring is src/tool-score.ts.
import { readFileSync, realpathSync } from 'node:fs'
import { relative } from 'node:path'
import { caliperNorm, VERDICTS, type Norm, type Verdict } from './oracle-compare.ts'
import { rdgNorm, rdtNorm } from './tool-norm.ts'
import {
  scoreReported,
  scoreTool,
  type Reported,
  type Stratum,
  type ToolScore,
  type UniverseProp,
} from './tool-score.ts'

export const normComponent = (name: string) =>
  (
    name
      .replace(/ \(.*\)$/, '')
      .split('/')
      .at(-1) ?? ''
  )
    .split('.')
    .at(-1)!
    .replaceAll(/[^A-Za-z0-9]/g, '')

export const pathKey = (path: string) =>
  process.platform === 'darwin' || process.platform === 'win32' ? path.toLowerCase() : path

/** The files of a component's collapsed inheritance layers: Caliper lists their props as members, not as props. */
export const collapsedLayerFiles = (component: any): Set<string> =>
  new Set(
    (component?.inheritance ?? [])
      .filter((layer: any) => layer.totalProps > 30)
      .map((layer: any) => pathKey(String(layer.fileName))),
  )

/** A component as the tools are matched to the checker's: the file and the normalised name. */
const componentKey = (file: string, name: string) => `${file}\0${normComponent(name)}`

export interface ToolDump {
  /** Props by component; a file with two components of one normalised name merges them, the first prop winning. */
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

type Entry = UniverseProp & { caliper: Reported; rdt?: Reported; rdg?: Reported }
interface Match {
  name: string
  key: string
  component: any
}

/** Where the checker declares a prop, by its first declaration: the component's own file, another file of the corpus, or
 * a dependency. */
function stratumOf(p: any, file: string): Stratum {
  const path = p.declarations?.[0]?.path
  if (!path) return 'unknown'
  if (String(path).includes('/node_modules/')) return 'dependency'
  return pathKey(String(path)) === pathKey(realpathSync(file)) ? 'same_file' : 'other_project_file'
}

/** What Caliper reports for one prop: the type, or why it has none. An ambiguous name is not scored for Caliper, as in
 * the verdicts of src/oracle.ts. */
function caliperReported(match: Match | undefined, p: any, caliper: any): Reported {
  const id = match?.component.props[p.name]
  if (id !== undefined) return caliperNorm(caliper.props[id].type)
  if (!match) return 'component'
  const layerFiles = collapsedLayerFiles(match.component)
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

/** Collects the universe as components are visited. */
export function universeRecorder(env: Environment) {
  const entries: Entry[] = []
  const rel = (path: string) => relative(env.root, realpathSync(path))
  return {
    entries,
    record({ own, name, file, matches }: { own: any[]; name: string; file: string; matches: Match[] }) {
      if (own.length === 0) return
      const key = componentKey(rel(file), name)
      const types = env.checker.getTypeOfSymbol(own)
      const [match] = matches.length === 1 ? matches : []
      for (const [i, p] of own.entries()) {
        entries.push({
          component: key,
          prop: p.name,
          stratum: stratumOf(p, file),
          checker: env.checkerNorm(types[i]),
          caliper: caliperReported(match, p, env.caliper),
          rdt: env.rdt && reported(env.rdt, { key, prop: p.name }, (t) => rdtNorm(t.typeObject)),
          rdg: env.rdg && reported(env.rdg, { key, prop: p.name }, (t) => rdgNorm(t.tsType)),
        })
      }
    },
    /** One score per tool. Caliper is scored over the universe as well as over the props both list (`counts`, the
     * verdicts of src/oracle.ts): the two must agree, or the universe is not the one the verdicts were made over. */
    score(counts: Record<Verdict, number>): Record<string, ToolScore & { failed_files?: number }> {
      const caliper = scoreTool(entries, (u) => u.caliper)
      for (const verdict of VERDICTS) {
        if (caliper.verdicts[verdict] !== counts[verdict]) {
          console.error(
            `warning: Caliper "${verdict}" is ${counts[verdict]} over the shared props, not ${caliper.verdicts[verdict]} over the universe`,
          )
        }
      }
      return {
        caliper,
        // Caliper over the components it reports only (src/tool-score.ts `scoreReported`): the weekly headline.
        caliper_reported: scoreReported(entries, (u) => u.caliper),
        ...(env.rdt && { rdt: scoreTool(entries, (u) => u.rdt!) }),
        ...(env.rdg && {
          react_docgen: { ...scoreTool(entries, (u) => u.rdg!), failed_files: env.rdg.failed.size },
        }),
      }
    },
  }
}
