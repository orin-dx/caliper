import type { Diagnostic } from 'caliper-docgen-napi'

export type DiagnosticLevel = 'silent' | 'error' | 'warn' | 'info'

interface Log {
  error(message: string): void
  warn(message: string): void
  info(message: string): void
}

const RANK: Record<DiagnosticLevel, number> = { silent: 0, error: 1, warn: 2, info: 3 }
const SEVERITY_RANK: Record<Diagnostic['severity'], number> = { error: 1, warning: 2, info: 3 }

function format(diagnostic: Diagnostic): string {
  const where = diagnostic.file
    ? ` ${diagnostic.file}${diagnostic.line === null ? '' : `:${diagnostic.line}${diagnostic.column === null ? '' : `:${diagnostic.column}`}`}`
    : ''
  return `[caliper-docgen]${where} ${diagnostic.message}${diagnostic.help ? ` (${diagnostic.help})` : ''}`
}

/**
 * Reports each extraction diagnostic once, however many revisions or full outputs repeat it, at or above `level`.
 * A diagnostic a later revision removed is reported again if it comes back.
 */
export class DiagnosticReporter {
  private readonly seen = new Set<string>()

  private readonly level: DiagnosticLevel
  private readonly log: Log
  private readonly onDiagnostic: ((diagnostic: Diagnostic) => void) | undefined

  constructor(level: DiagnosticLevel, log: Log, onDiagnostic?: (diagnostic: Diagnostic) => void) {
    this.level = level
    this.log = log
    this.onDiagnostic = onDiagnostic
  }

  report(diagnostics: Diagnostic[], removed: Diagnostic[] = []): void {
    for (const diagnostic of removed) this.seen.delete(JSON.stringify(diagnostic))
    for (const diagnostic of diagnostics) {
      const id = JSON.stringify(diagnostic)
      if (this.seen.has(id)) continue
      this.seen.add(id)
      this.onDiagnostic?.(diagnostic)
      if (SEVERITY_RANK[diagnostic.severity] > RANK[this.level]) continue
      const { severity } = diagnostic
      this.log[severity === 'warning' ? 'warn' : severity](format(diagnostic))
    }
  }
}
