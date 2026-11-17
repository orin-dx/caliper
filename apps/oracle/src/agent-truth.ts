// What TypeScript 7.1's checker says about the questions in scripts/agent_questions.json, for scripts/agent_eval.py.
//
// The weekly agent evaluation asks what a reader of Caliper's output would answer about one component and compares that
// with the checker, never with Caliper itself. This script is the checker's side: for each question it finds the
// component's exported props type the way src/oracle.ts does and writes one fact per question id. A question the checker
// cannot answer (no such component, no such prop, a prop that is not a literal union) is `invalid`, which the evaluation
// reports apart from the answers it scores.
//
// usage: tsx src/agent-truth.ts --root DIR --tsconfig FILE --questions FILE --corpus NAME --out FILE
import { readFileSync, realpathSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { API, SignatureKind, SymbolFlags, TypeFlags } from 'typescript-beta/sync'
import { declaredName, listedName, propsTypeOf } from './checker-components.ts'

const KEYS = ['root', 'tsconfig', 'questions', 'corpus', 'out'] as const
const args: Partial<Record<(typeof KEYS)[number], string>> = {}
for (let i = 2; i < process.argv.length; i += 2) {
  args[process.argv[i].replace(/^--/, '') as (typeof KEYS)[number]] = process.argv[i + 1]
}
if (!args.root || !args.tsconfig || !args.questions || !args.corpus || !args.out) {
  console.error('usage: tsx src/agent-truth.ts --root DIR --tsconfig FILE --questions FILE --corpus NAME --out FILE')
  process.exit(2)
}

interface Question {
  id: string
  corpus: string
  kind: 'present' | 'prop_exists' | 'prop_required' | 'literal_values' | 'type_kind'
  file: string
  component: string
  prop?: string
}
type Truth = { value: boolean | string | string[] } | { invalid: string }

const root = realpathSync(args.root)
const questions = (JSON.parse(readFileSync(args.questions, 'utf8')).questions as Question[]).filter(
  (q) => q.corpus === args.corpus,
)
const absolute = (q: Question) => realpathSync(join(root, q.file))
const files = [...new Set(questions.map((q) => absolute(q)))]

const api = new API({ cwd: root })
// The component files only, with the tsconfig the oracle uses; the checker reads what they import on demand.
const { options } = api.parseConfigFile(args.tsconfig)
const program = api.createProgram(files, options)
const { checker } = program.getProject()

const without = (type: any): any => checker.getNonNullableType(type)

/** What a reader of one prop's type could say about it, from the checker's type and not from its text. */
function kindOf(type: any): string {
  const t = without(type)
  if (t.flags & TypeFlags.Boolean) return 'boolean'
  if (t.flags & TypeFlags.String) return 'string'
  if (t.flags & TypeFlags.Number) return 'number'
  if (t.flags & TypeFlags.Union) {
    const members: any[] = t.getTypes() ?? []
    if (members.length > 0 && members.every((m) => m.flags & TypeFlags.StringLiteral)) return 'literal_union'
    return 'other'
  }
  if (t.flags & TypeFlags.StringLiteral) return 'literal_union'
  if (t.flags & TypeFlags.Object) {
    if (checker.isArrayType(t)) return 'array'
    const [sig] = checker.getSignaturesOfType(t, SignatureKind.Call)
    if (sig && checker.getPropertiesOfType(t).length === 0) return 'function'
  }
  return 'other'
}

function literalValues(type: any): string[] | undefined {
  const t = without(type)
  const members: any[] = t.flags & TypeFlags.Union ? (t.getTypes() ?? []) : [t]
  if (members.length === 0 || !members.every((m) => m.flags & TypeFlags.StringLiteral)) return undefined
  return members.map((m) => String(m.value)).toSorted()
}

function answer(q: Question): Truth {
  const moduleSymbol = checker.getSymbolOfSourceFile(absolute(q))
  if (!moduleSymbol) return { invalid: `the checker has no module for ${q.file}` }
  const exported = checker
    .getExportsOfModule(moduleSymbol)
    .find((e: any) => listedName(e.name, declaredName(checker, e), absolute(q)) === q.component)
  let props: any[] = []
  if (exported) {
    // A checker failure propagates to the caller's `the checker failed` fact instead of reading as no props.
    const propsType = propsTypeOf(checker, exported)
    props = propsType ? [...checker.getPropertiesOfType(propsType)] : []
  }
  if (q.kind === 'present') return { value: props.length > 0 }
  if (props.length === 0) return { invalid: `the checker finds no component ${q.component} in ${q.file}` }
  const prop = props.find((p: any) => p.name === q.prop)
  if (q.kind === 'prop_exists') return { value: prop !== undefined }
  if (!prop) return { invalid: `the checker finds no prop ${q.prop} on ${q.component}` }
  if (q.kind === 'prop_required') return { value: !(prop.flags & SymbolFlags.Optional) }
  const type = checker.getTypeOfSymbol(prop)
  if (q.kind === 'type_kind') return { value: kindOf(type) }
  const values = literalValues(type)
  return values
    ? { value: values }
    : { invalid: `${q.prop} is ${checker.typeToString(type)}, not a string literal union` }
}

const truth: Record<string, Truth> = {}
for (const q of questions) {
  try {
    truth[q.id] = answer(q)
  } catch (error) {
    truth[q.id] = { invalid: `the checker failed: ${String(error).slice(0, 120)}` }
  }
}
writeFileSync(args.out, JSON.stringify(truth, null, 2))
program.dispose()
api.close()
console.error(`${questions.length} questions, ${files.length} files`)
