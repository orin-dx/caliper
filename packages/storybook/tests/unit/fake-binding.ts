import type {
  ComponentEntry,
  Diagnostic,
  ExtractionOutput,
  IncrementalUpdate,
  RdtComponentDoc,
  WireDiagnostic,
} from 'caliper-docgen-napi'
import { SUPPORTED_SCHEMA_VERSION } from '../../src/schema.js'
import type { SessionBinding } from '../../src/session.js'

export function entry(displayName: string, filePath: string): ComponentEntry {
  return {
    displayName,
    filePath,
    description: '',
    props: {},
    inheritance: [],
    notableInherited: {},
    discriminantProp: null,
    composes: [],
    tags: {},
    methods: [],
    detectedBy: 'functionDeclaration',
  }
}

export function rdtDoc(displayName: string, propNames: string[] = []): RdtComponentDoc {
  return {
    displayName,
    description: `${displayName} docs.`,
    props: Object.fromEntries(
      propNames.map((name) => [
        name,
        { name, type: { name: 'string' }, required: false, defaultValue: null, description: '', parent: null },
      ]),
    ),
    methods: [],
    tags: {},
    composes: [],
  }
}

export function warning(affects: string[], severity: Diagnostic['severity'] = 'warning'): Diagnostic {
  return {
    severity,
    message: 'Cannot resolve type',
    file: null,
    line: null,
    column: null,
    help: null,
    code: 'x',
    affects,
  }
}

type WithDiagnostics<T> = Omit<T, 'diagnostics' | 'removedDiagnostics'> & {
  diagnostics?: Diagnostic[]
  removedDiagnostics?: Diagnostic[]
}

/** Writes `diagnostics` as the binding does: each `affects` list is an entry of `lists`, under an id. */
function wire(diagnostics: Diagnostic[] = [], lists: Record<string, string[]>): WireDiagnostic[] {
  return diagnostics.map(({ affects, ...rest }) => {
    if (!affects?.length) return rest
    const id = affects.join('|')
    lists[id] = affects
    return { ...rest, affects: id }
  })
}

/** A stand-in for the native binding: a fixed cold index, then one queued update per look at the files. */
export class FakeBinding implements SessionBinding {
  /** How many times the files were looked at. */
  looks = 0
  /** The updates the next looks return, in order; a look with none queued finds nothing changed. */
  readonly updates: Partial<WithDiagnostics<IncrementalUpdate>>[] = []
  docs: Record<string, RdtComponentDoc> = {}
  /** Runs while a look is under way, before its update is returned. */
  duringLook: (() => void) | undefined
  initializations = 0
  closed = false
  private revision = 1

  constructor(public output: Partial<WithDiagnostics<ExtractionOutput>>) {}

  createSession = (): number => 7

  initializeSessionIndex = async (): Promise<string> => {
    this.initializations++
    const nameLists: Record<string, string[]> = {}
    const { diagnostics, ...rest } = this.output
    return JSON.stringify({
      schemaVersion: SUPPORTED_SCHEMA_VERSION,
      components: {},
      nameLists,
      diagnostics: wire(diagnostics, nameLists),
      ...rest,
    })
  }

  ensureFresh = async (): Promise<string> => {
    this.looks++
    const during = this.duringLook
    this.duringLook = undefined
    during?.()
    await Promise.resolve()
    const next = this.updates.shift()
    const nameLists: Record<string, string[]> = {}
    const { diagnostics, removedDiagnostics, ...rest } = next ?? {}
    const update: IncrementalUpdate = {
      schemaVersion: SUPPORTED_SCHEMA_VERSION,
      revision: next ? ++this.revision : this.revision,
      updatedComponents: [],
      props: {},
      propSets: {},
      updatedKeys: [],
      removedKeys: [],
      affectedFiles: [],
      durationMs: 0,
      ...rest,
      removedDiagnostics: wire(removedDiagnostics, nameLists),
      diagnostics: wire(diagnostics, nameLists),
      nameLists,
    }
    return JSON.stringify(update)
  }

  sessionDocgenInfo = (_sessionId: number, keys: string[]): string =>
    JSON.stringify(Object.fromEntries(keys.filter((key) => key in this.docs).map((key) => [key, this.docs[key]])))

  closeSession = (): void => {
    this.closed = true
  }
}
