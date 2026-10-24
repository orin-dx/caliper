import type {
  ComponentEntry,
  Diagnostic,
  ExtractionOutput,
  IncrementalUpdate,
  RdtComponentDoc,
} from 'caliper-docgen-napi'
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

/** A stand-in for the native binding: a fixed cold index, then one queued update per look at the files. */
export class FakeBinding implements SessionBinding {
  /** How many times the files were looked at. */
  looks = 0
  /** The updates the next looks return, in order; a look with none queued finds nothing changed. */
  readonly updates: Partial<IncrementalUpdate>[] = []
  docs: Record<string, RdtComponentDoc> = {}
  /** Runs while a look is under way, before its update is returned. */
  duringLook: (() => void) | undefined
  initializations = 0
  closed = false
  private revision = 1

  constructor(public output: Partial<ExtractionOutput>) {}

  createSession = (): number => 7

  initializeSessionIndex = async (): Promise<string> => {
    this.initializations++
    return JSON.stringify({ components: {}, diagnostics: [], ...this.output })
  }

  ensureFresh = async (): Promise<string> => {
    this.looks++
    const during = this.duringLook
    this.duringLook = undefined
    during?.()
    await Promise.resolve()
    const next = this.updates.shift()
    const update: IncrementalUpdate = {
      revision: next ? ++this.revision : this.revision,
      updatedComponents: [],
      props: {},
      updatedKeys: [],
      removedKeys: [],
      removedDiagnostics: [],
      affectedFiles: [],
      diagnostics: [],
      durationMs: 0,
      ...next,
    }
    return JSON.stringify(update)
  }

  sessionDocgenInfo = (_sessionId: number, keys: string[]): string =>
    JSON.stringify(Object.fromEntries(keys.filter((key) => key in this.docs).map((key) => [key, this.docs[key]])))

  /** Counts the components a lazy session was asked to resolve. */
  resolved: string[] = []

  sessionComponentAnswers = (_sessionId: number, keys: string[]): string => {
    this.resolved.push(...keys)
    const diagnostics = this.output.diagnostics ?? []
    return JSON.stringify(
      Object.fromEntries(
        keys
          .filter((key) => key in this.docs)
          .map((key) => [
            key,
            { docgenInfo: this.docs[key], diagnostics: diagnostics.filter((each) => each.affects?.includes(key)) },
          ]),
      ),
    )
  }

  /** The sessions asked to warm up, with how many components. */
  warmUps: number[] = []

  sessionWarmUp = async (_sessionId: number, count: number): Promise<number> => {
    this.warmUps.push(count)
    return count
  }

  closeSession = (): void => {
    this.closed = true
  }
}
