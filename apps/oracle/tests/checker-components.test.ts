// How a module's components are read from TypeScript 7.1's checker: once per declaration, with every name the module
// exports it under, and the name a tool that has no exports lists it by. The checker names the function type's symbol of a
// default export `default` and gives the declaration handle no name of its own.
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { API } from 'typescript-beta/sync'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { declaredExports, listedName, propsTypeOf } from '../src/checker-components.ts'

const FILES: Record<string, string> = {
  'Badge.tsx': `export default function Badge(props: { label: string }) { return <span>{props.label}</span> }\n`,
  'Card.tsx': `const Card = (props: { title: string }) => <b>{props.title}</b>\nexport default Card\n`,
  'Panel.tsx': `export default class Panel { props!: { open: boolean } }\n`,
  'button-group.tsx': `export default function (props: { label: string }) { return <i>{props.label}</i> }\n`,
  'Wrapped.tsx': `export default (props: { label: string }) => <u>{props.label}</u>\n`,
  'Reexport.tsx': `export { default } from './Badge'\n`,
  'Named.tsx': `export function Named(props: { label: string }) { return <u>{props.label}</u> }\n`,
  'Twice.tsx': `export default function Twice(props: { label: string }) { return <u>{props.label}</u> }\nexport { Twice }\n`,
  'Aliased.tsx': `function Chip(props: { label: string }) { return <u>{props.label}</u> }\nexport { Chip, Chip as Tag }\n`,
  'AliasedDefault.tsx': `const Pill = (props: { label: string }) => <u>{props.label}</u>\nexport default Pill\nexport { Pill as Lozenge }\n`,
  'One.tsx': `export function Item(props: { one: string }) { return <u>{props.one}</u> }\n`,
  'Two.tsx': `export function Item(props: { two: string }) { return <u>{props.two}</u> }\n`,
  'Underscore.tsx': `export function _Card(props: { a: string }) { return <u>{props.a}</u> }\nexport function Card(props: { b: string }) { return <u>{props.b}</u> }\n`,
  'Renamed.tsx': `function CardImpl(props: { title: string }) { return <u>{props.title}</u> }\nexport { CardImpl as Card }\n`,
  'Barrel.tsx': `export { Item } from './One'\nexport { Item as Other } from './Two'\nexport * from './Named'\n`,
}

let dir: string
let listed: Map<string, string[]>
let propNames: Map<string, string[]>
let exportedAs: Map<string, string[][]>

beforeAll(() => {
  dir = realpathSync(mkdtempSync(join(tmpdir(), 'checker-components-')))
  mkdirSync(join(dir, 'src'))
  for (const [file, text] of Object.entries(FILES)) writeFileSync(join(dir, 'src', file), text)
  writeFileSync(
    join(dir, 'tsconfig.json'),
    JSON.stringify({
      compilerOptions: {
        jsx: 'preserve',
        strict: true,
        target: 'es2022',
        module: 'esnext',
        moduleResolution: 'bundler',
      },
    }),
  )
  const api = new API({ cwd: dir })
  const { options } = api.parseConfigFile(join(dir, 'tsconfig.json'))
  const files = Object.keys(FILES).map((f) => join(dir, 'src', f))
  const program = api.createProgram(files, options)
  const { checker } = program.getProject()
  listed = new Map()
  propNames = new Map()
  exportedAs = new Map()
  for (const file of Object.keys(FILES)) {
    const path = join(dir, 'src', file)
    const moduleSymbol = checker.getSymbolOfSourceFile(path)!
    const exported = checker.getExportsOfModule(moduleSymbol)
    const declarations = declaredExports(checker, moduleSymbol)
    exportedAs.set(
      file,
      declarations.map((d) => d.names),
    )
    listed.set(
      file,
      declarations.map((d) => listedName(d.exported.name, d.declared, path)),
    )
    propNames.set(
      file,
      exported.flatMap((e: any) => {
        const type = propsTypeOf(checker, e)
        return type ? checker.getPropertiesOfType(type).map((p: any) => p.name) : []
      }),
    )
  }
  program.dispose()
  api.close()
}, 120_000)

afterAll(() => rmSync(dir, { recursive: true, force: true }))

describe('listedName', () => {
  it('names a default-exported function by its declaration', () => {
    expect(listed.get('Badge.tsx')).toStrictEqual(['Badge'])
  })

  it('names a default-exported alias by what it exports', () => {
    expect(listed.get('Card.tsx')).toStrictEqual(['Card'])
  })

  it('names a default-exported class by its declaration', () => {
    expect(listed.get('Panel.tsx')).toStrictEqual(['Panel'])
  })

  it('names an anonymous default export after its file as react-docgen-typescript does, not as Caliper does', () => {
    expect(listed.get('button-group.tsx')).toStrictEqual(['button-group'])
    expect(listed.get('Wrapped.tsx')).toStrictEqual(['Wrapped'])
  })

  it('leaves a named export as it is, and an export under another name by that name', () => {
    expect(listed.get('Named.tsx')).toStrictEqual(['Named'])
    expect(listed.get('Aliased.tsx')).toStrictEqual(['Chip'])
  })

  it('still finds the props of every one of them', () => {
    expect(propNames.get('Badge.tsx')).toStrictEqual(['label'])
    expect(propNames.get('button-group.tsx')).toStrictEqual(['label'])
  })
})

describe('declaredExports', () => {
  it('counts a function exported as default and by name once, under both names', () => {
    expect(exportedAs.get('Twice.tsx')).toStrictEqual([['default', 'Twice']])
  })

  it('counts a declaration exported under an alias once, under every name Caliper lists for it', () => {
    expect(exportedAs.get('Aliased.tsx')).toStrictEqual([['Chip', 'Tag']])
  })

  it('counts a default export and an alias of the same declaration once', () => {
    expect(exportedAs.get('AliasedDefault.tsx')).toStrictEqual([['default', 'Lozenge']])
  })

  it('counts two components of one name in different files both', () => {
    expect(exportedAs.get('One.tsx')).toStrictEqual([['Item']])
    expect(exportedAs.get('Two.tsx')).toStrictEqual([['Item']])
  })

  it('leaves a re-export to the file that declares the component', () => {
    expect(exportedAs.get('Reexport.tsx')).toStrictEqual([])
    expect(exportedAs.get('Barrel.tsx')).toStrictEqual([])
  })

  it('keeps an ordinary file as it is', () => {
    expect(exportedAs.get('Named.tsx')).toStrictEqual([['Named']])
    expect(exportedAs.get('Badge.tsx')).toStrictEqual([['default']])
  })

  it('counts _Card and Card, two components of one file, as two', () => {
    expect(exportedAs.get('Underscore.tsx')).toStrictEqual([['_Card'], ['Card']])
    expect(listed.get('Underscore.tsx')).toStrictEqual(['_Card', 'Card'])
  })

  it('names an export of another name by the export, which is what Caliper lists', () => {
    expect(exportedAs.get('Renamed.tsx')).toStrictEqual([['Card']])
  })
})
