// The name a default export is listed under, read from TypeScript 7.1's checker, which names the function type's symbol
// `default` and gives the declaration handle no name of its own.
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { API } from 'typescript-beta/sync'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { exportName, propsTypeOf } from '../src/checker-components.ts'

const FILES: Record<string, string> = {
  'Badge.tsx': `export default function Badge(props: { label: string }) { return <span>{props.label}</span> }\n`,
  'Card.tsx': `const Card = (props: { title: string }) => <b>{props.title}</b>\nexport default Card\n`,
  'Panel.tsx': `export default class Panel { props!: { open: boolean } }\n`,
  'button-group.tsx': `export default function (props: { label: string }) { return <i>{props.label}</i> }\n`,
  'Wrapped.tsx': `export default (props: { label: string }) => <u>{props.label}</u>\n`,
  'Reexport.tsx': `export { default } from './Badge'\n`,
  'Named.tsx': `export function Named(props: { label: string }) { return <u>{props.label}</u> }\n`,
}

let dir: string
let names: Map<string, string[]>
let propNames: Map<string, string[]>

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
  for (const file of Object.keys(FILES)) {
    const moduleSymbol = checker.getSymbolOfSourceFile(join(dir, 'src', file))!
    const exported = checker.getExportsOfModule(moduleSymbol)
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
