// The name a default export is listed under, read from TypeScript 7.1's checker, which names the function type's symbol
// `default` and gives the declaration handle no name of its own.
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { API } from 'typescript-beta/sync'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { declaredExports, exportName, propsTypeOf } from '../src/checker-components.ts'

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
  'Barrel.tsx': `export { Item } from './One'\nexport { Item as Other } from './Two'\nexport * from './Named'\n`,
}

let dir: string
let names: Map<string, string[]>
let propNames: Map<string, string[]>
let declared: Map<string, string[]>

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
  names = new Map()
  propNames = new Map()
  declared = new Map()
  for (const file of Object.keys(FILES)) {
    const moduleSymbol = checker.getSymbolOfSourceFile(join(dir, 'src', file))!
    const exported = checker.getExportsOfModule(moduleSymbol)
    declared.set(
      file,
      declaredExports(checker, moduleSymbol).map((e: any) => exportName(checker, e)),
    )
    names.set(
      file,
      exported.map((e: any) => exportName(checker, e)),
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

describe('exportName', () => {
  it('names a default-exported function by its declaration', () => {
    expect(names.get('Badge.tsx')).toStrictEqual(['Badge'])
  })

  it('names a default-exported alias by what it exports', () => {
    expect(names.get('Card.tsx')).toStrictEqual(['Card'])
  })

  it('names a default-exported class by its declaration', () => {
    expect(names.get('Panel.tsx')).toStrictEqual(['Panel'])
  })

  it('names an anonymous default export after its file, PascalCased as Caliper does', () => {
    expect(names.get('button-group.tsx')).toStrictEqual(['ButtonGroup'])
    expect(names.get('Wrapped.tsx')).toStrictEqual(['Wrapped'])
  })

  it('follows a re-exported default to the declaration', () => {
    expect(names.get('Reexport.tsx')).toStrictEqual(['Badge'])
  })

  it('leaves a named export as it is', () => {
    expect(names.get('Named.tsx')).toStrictEqual(['Named'])
  })

  it('still finds the props of every one of them', () => {
    expect(propNames.get('Badge.tsx')).toStrictEqual(['label'])
    expect(propNames.get('button-group.tsx')).toStrictEqual(['label'])
  })
})

describe('declaredExports', () => {
  it('counts a function exported as default and by name once', () => {
    expect(names.get('Twice.tsx')).toStrictEqual(['Twice', 'Twice'])
    expect(declared.get('Twice.tsx')).toStrictEqual(['Twice'])
  })

  it('counts a declaration exported under an alias once, under its own name', () => {
    expect(declared.get('Aliased.tsx')).toStrictEqual(['Chip'])
  })

  it('counts a default export and an alias of the same declaration once', () => {
    expect(declared.get('AliasedDefault.tsx')).toStrictEqual(['Pill'])
  })

  it('counts two components of one name in different files both', () => {
    expect(declared.get('One.tsx')).toStrictEqual(['Item'])
    expect(declared.get('Two.tsx')).toStrictEqual(['Item'])
  })

  it('leaves a re-export to the file that declares the component', () => {
    expect(declared.get('Reexport.tsx')).toStrictEqual([])
    expect(declared.get('Barrel.tsx')).toStrictEqual([])
  })

  it('keeps an ordinary file as it is', () => {
    expect(declared.get('Named.tsx')).toStrictEqual(['Named'])
    expect(declared.get('Badge.tsx')).toStrictEqual(['Badge'])
  })
})
