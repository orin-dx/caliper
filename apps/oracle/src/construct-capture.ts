// Writes (or, with --check, verifies) the golden files of the deterministic construct suite: for every fixture under
// construct-suite/, the type TypeScript 7.1's checker gives prop `p` of each row's component (the exported `Row`, or
// the components a fixture lists), normalized as src/oracle.ts normalizes it, and the facts about the prop both lanes
// are held to (`Facts`). One program holds all fixtures.
import { existsSync, readFileSync, realpathSync, writeFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { SyntaxKind } from 'typescript-beta/ast'
import { API, SymbolFlags } from 'typescript-beta/sync'
import { exportAt, propsTypeOf } from './checker-components.ts'
import { listFixtures, PROP, type Facts, type Fixture, type Golden } from './construct-suite.ts'
import { checkerNormalizer } from './oracle-checker.ts'

const suite = realpathSync(join(import.meta.dirname, '..', 'construct-suite'))
const check = process.argv.includes('--check')

const fixtures = listFixtures(suite)
if (fixtures.length === 0) throw new Error(`no fixtures under ${suite}`)

const api = new API({ cwd: suite })
// `parseConfigFile`, not options by hand: the 7.1 beta's server panics on string-valued enum options (src/oracle.ts).
const { options } = api.parseConfigFile(join(suite, 'tsconfig.json'))
const program = api.createProgram([...new Set(fixtures.map((f) => f.file))], options)
const { checker } = program.getProject()
const norm = checkerNormalizer(checker)

/** The union a declaration writes for its type: the type node itself, or the type of the non-generic alias it names. */
function writtenUnion(typeNode: any): any[] | null {
  let node = typeNode
  if (node?.kind === SyntaxKind.TypeReference) {
    const alias = checker.getSymbolAtLocation(node.typeName)
    const declaration: any =
      alias && alias.flags & SymbolFlags.TypeAlias ? alias.declarations?.[0]?.resolve() : undefined
    if (declaration?.kind === SyntaxKind.TypeAliasDeclaration && declaration.typeParameters === undefined) {
      node = declaration.type
    }
  }
  return node?.kind === SyntaxKind.UnionType ? [...node.types] : null
}

/** The default the component's own code gives prop `p`, as written: the initializer of `p` in the object pattern of its
 * first parameter, else the value of `p` in a `<name>.defaultProps = { … }` statement of its file. Null for neither. */
function codeDefault(exported: any): string | null {
  const symbol = exported.flags & SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported
  const declaration: any = symbol.declarations?.[0]?.resolve()
  const pattern = declaration?.parameters?.[0]?.name
  if (pattern?.kind === SyntaxKind.ObjectBindingPattern) {
    for (const element of pattern.elements) {
      if ((element.propertyName ?? element.name)?.text === PROP && element.initializer) {
        return element.initializer.getText()
      }
    }
  }
  const name = declaration?.name?.text
  for (const statement of declaration?.getSourceFile()?.statements ?? []) {
    const assigned = statement.kind === SyntaxKind.ExpressionStatement ? statement.expression : undefined
    const target = assigned?.kind === SyntaxKind.BinaryExpression ? assigned.left : undefined
    if (
      target?.kind !== SyntaxKind.PropertyAccessExpression ||
      target.name.text !== 'defaultProps' ||
      target.expression.text !== name ||
      assigned.right.kind !== SyntaxKind.ObjectLiteralExpression
    ) {
      continue
    }
    for (const property of assigned.right.properties) {
      if (property.kind === SyntaxKind.PropertyAssignment && property.name.text === PROP) {
        return property.initializer.getText()
      }
    }
  }
  return null
}

function factsOf(prop: any, exported: any): Facts {
  const tags: Record<string, string> = {}
  for (const tag of checker.getJsDocTagsOfSymbol(prop)) tags[tag.name] = (tag.text ?? '').trim()
  const declaration: any = prop.declarations?.[0]?.resolve()
  const parent = declaration?.parent
  const members = writtenUnion(declaration?.type)
  return {
    description: checker.getDocumentationCommentOfSymbol(prop),
    tags,
    default: codeDefault(exported) ?? tags.default ?? tags.defaultValue ?? null,
    required: (prop.flags & SymbolFlags.Optional) === 0,
    parent:
      parent?.kind === SyntaxKind.InterfaceDeclaration
        ? { name: parent.name.text, file: relative(suite, realpathSync(parent.getSourceFile().fileName)) }
        : null,
    order: members && members.map((member) => norm(checker.getTypeFromTypeNode(member))),
  }
}

function capture(fixture: Fixture): Golden {
  const moduleSymbol = checker.getSymbolOfSourceFile(fixture.file)
  const exported = moduleSymbol && exportAt(checker, moduleSymbol, fixture.exportPath)
  if (!exported) throw new Error(`${fixture.id}: no export ${fixture.exportPath.join('.')}`)
  const propsType = propsTypeOf(checker, exported)
  const prop = propsType && checker.getPropertyOfType(propsType, PROP)
  if (!prop) throw new Error(`${fixture.id}: ${fixture.component} has no prop ${PROP}`)
  const type = checker.getTypeOfSymbol(prop)
  return { checker: norm(type), text: checker.typeToString(type), facts: factsOf(prop, exported) }
}

// A unique-symbol key is named `__@tag@<symbol id>` and the id depends on the other files in the program, so adding a
// fixture would change an unrelated golden. Keep the name, drop the id.
const stableSymbolIds = (json: string) => json.replaceAll(/__@(\w+)@\d+/g, '__@$1')

const stale: string[] = []
for (const fixture of fixtures) {
  const text = `${stableSymbolIds(JSON.stringify(capture(fixture), null, 2))}\n`
  const current = existsSync(fixture.golden) ? readFileSync(fixture.golden, 'utf8') : undefined
  if (current === text) continue
  stale.push(fixture.id)
  if (!check) writeFileSync(fixture.golden, text)
}
program.dispose()
api.close()

if (check && stale.length > 0) {
  console.error(
    `golden files out of date (run pnpm --filter caliper-docgen-oracle construct:capture):\n  ${stale.join('\n  ')}`,
  )
  process.exit(1)
}
console.error(`${fixtures.length} fixtures, ${stale.length} ${check ? 'differ' : 'written'}`)
