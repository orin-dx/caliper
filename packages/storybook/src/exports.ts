// What a module's source says its exports stand for, read only when a story's import does not match a component by name.

/**
 * Whether the source exports a value declared under `name`: `export function Name`, `export const Name`,
 * `export class Name` or `export { Name }`.
 */
export function declaresExport(source: string, name: string): boolean {
  const escaped = name.replaceAll('$', String.raw`\$`)
  const declared = new RegExp(`export\\s+(?:async\\s+)?(?:function\\s*\\*?|class|const|let|var)\\s+${escaped}\\b`)
  const listed = new RegExp(`export\\s*\\{[^}]*(?<![\\w$])${escaped}(?![\\w$])(?!\\s+as\\b)[^}]*\\}`)
  return declared.test(source) || listed.test(source)
}

/**
 * The local name `exportName` stands for: `export default Card` and `export default forwardRef(Card)` give `Card`,
 * `export { Inner as Card }` gives `Inner`. Undefined when the source does not say.
 */
export function localNameOf(source: string, exportName: string): string | undefined {
  const alias = new RegExp(
    `export\\s*\\{[^}]*?\\b(\\w+)\\s+as\\s+${exportName.replaceAll('$', String.raw`\$`)}\\b[^}]*\\}`,
  ).exec(source)
  if (alias) return alias[1]
  if (exportName !== 'default') return undefined
  const declared = /export\s+default\s+(?:async\s+)?(?:function\s*\*?|class)\s+(\w+)/.exec(source)
  if (declared) return declared[1]
  const wrapped = /export\s+default\s+(?:[\w.]+\s*(?:<[^>(]*>)?\(\s*)+(\w+)/.exec(source)
  if (wrapped) return wrapped[1]
  return /export\s+default\s+(\w+)\s*(?:as\s+[\w.]+)?\s*;?\s*$/m.exec(source)?.[1]
}
