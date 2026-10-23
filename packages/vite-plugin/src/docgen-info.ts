import type { ExtractionOutput, IncrementalUpdate, RdtComponentDoc } from 'caliper-docgen-napi'
import type { EnvironmentModuleNode } from 'vite'

/** A component's name as code can reach it: an identifier, or a member path like `List.Item`. */
const REACHABLE_NAME = /^[A-Za-z_$][\w$]*(\.[A-Za-z_$][\w$]*)*$/

/** Which file declares each output key, kept in step with the session for `injectDocgenInfo`. */
export class ComponentFiles {
  private readonly keyFiles = new Map<string, string>()

  index(output: ExtractionOutput): void {
    this.keyFiles.clear()
    for (const [key, entry] of Object.entries(output.components)) this.keyFiles.set(key, entry.filePath)
  }

  apply(update: IncrementalUpdate): void {
    for (const key of update.removedKeys) this.keyFiles.delete(key)
    update.updatedKeys.forEach((key, i) => {
      const entry = update.updatedComponents[i]
      if (entry) this.keyFiles.set(key, entry.filePath)
    })
  }

  keysIn(file: string): string[] {
    return [...this.keyFiles].filter(([, path]) => path === file).map(([key]) => key)
  }
}

/** One `__docgenInfo` assignment per reachable component, each guarded so a name the module lacks is skipped. */
export function docgenAssignments(docs: Record<string, RdtComponentDoc>): string | undefined {
  const lines = Object.values(docs)
    .filter((doc) => REACHABLE_NAME.test(doc.displayName))
    .map((doc) => `try { ${doc.displayName}.__docgenInfo = ${JSON.stringify(doc)} } catch {}`)
  return lines.length > 0 ? lines.join('\n') : undefined
}

interface ModuleGraph {
  getModulesByFile(file: string): Set<EnvironmentModuleNode> | undefined
  invalidateModule(mod: EnvironmentModuleNode): void
}

/** Invalidates and returns the modules declaring components `update` changed, other than `changedFile`'s own. */
export function invalidateDeclaringModules(
  moduleGraph: ModuleGraph,
  update: IncrementalUpdate,
  changedFile: string,
): EnvironmentModuleNode[] {
  const files = new Set(update.updatedComponents.map((entry) => entry.filePath))
  files.delete(changedFile)
  const modules: EnvironmentModuleNode[] = []
  for (const file of files) {
    for (const mod of moduleGraph.getModulesByFile(file) ?? []) {
      moduleGraph.invalidateModule(mod)
      modules.push(mod)
    }
  }
  return modules
}
