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

/** A stand-in for the native binding: a fixed cold output, then one queued update per changed file. */
export class FakeBinding implements SessionBinding {
  readonly changed: string[] = []
  readonly updates = new Map<string, Partial<IncrementalUpdate>>()
  docs: Record<string, RdtComponentDoc> = {}
  /** Runs while a change is being extracted, before its update is returned. */
  duringExtraction: (() => void) | undefined
  initializations = 0
  closed = false
  private revision = 1

  constructor(public output: Partial<ExtractionOutput>) {}

  createSession = (): number => 7

  initializeSession = async (): Promise<string> => {
    this.initializations++
    return JSON.stringify({ components: {}, props: {}, enums: {}, diagnostics: [], ...this.output })
  }

  extractFileIncremental = async (filePath: string): Promise<string> => {
    this.changed.push(filePath)
    const during = this.duringExtraction
    this.duringExtraction = undefined
    during?.()
    await Promise.resolve()
    const update: IncrementalUpdate = {
      revision: ++this.revision,
      updatedComponents: [],
      props: {},
      updatedKeys: [],
      removedKeys: [],
      removedDiagnostics: [],
      affectedFiles: [filePath],
      diagnostics: [],
      durationMs: 0,
      ...this.updates.get(filePath),
    }
    return JSON.stringify(update)
  }

  sessionDocgenInfo = (_sessionId: number, keys: string[]): string =>
    JSON.stringify(Object.fromEntries(keys.filter((key) => key in this.docs).map((key) => [key, this.docs[key]])))

  closeSession = (): void => {
    this.closed = true
  }
}
