import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import { SUPPORTED_SCHEMA_VERSION } from '../../src/schema.js'
import { DocgenSession } from '../../src/session.js'
import { FakeBinding, entry, rdtDoc, warning } from './fake-binding.js'

const src = join('/project', 'src')
// Modules whose source the session reads to tell which component an export names.
const real = mkdtempSync(join(tmpdir(), 'caliper-sb-session-'))
afterAll(() => rmSync(real, { recursive: true, force: true }))

describe('DocgenSession.find', () => {
  function sessionWith(components: Record<string, ReturnType<typeof entry>>): Promise<DocgenSession> {
    const session = new DocgenSession({ srcDirs: [src] }, new FakeBinding({ components }))
    return session.fresh().then(() => session)
  }

  it('picks the component the export names in its file', async () => {
    const session = await sessionWith({
      List: entry('List', join(src, 'List.tsx')),
      ListItem: entry('ListItem', join(src, 'List.tsx')),
    })
    expect(session.keyOf(join(src, 'List.tsx'), 'ListItem')).toBe('ListItem')
    expect(session.keyOf(join(src, 'List.tsx'), 'default')).toBeUndefined()
  })

  it("answers a default export with the file's only component", async () => {
    const session = await sessionWith({ Card: entry('Card', join(src, 'Card.tsx')) })
    expect(session.keyOf(join(src, 'Card.tsx'), 'default')).toBe('Card')
  })

  it('follows a barrel to the one component with the exported name, and gives up when two share it', async () => {
    const session = await sessionWith({
      Button: entry('Button', join(src, 'Button.tsx')),
      'Tab (a/Tab.tsx)': entry('Tab', join(src, 'a', 'Tab.tsx')),
      'Tab (b/Tab.tsx)': entry('Tab', join(src, 'b', 'Tab.tsx')),
    })
    expect(session.keyOf(join(src, 'index.ts'), 'Button')).toBe('Button')
    expect(session.keyOf(join(src, 'index.ts'), 'Tab')).toBeUndefined()
  })

  it("answers a default export with the component the module's source exports, among several in the file", async () => {
    writeFileSync(join(real, 'Card.tsx'), 'const ActionNode = () => null\nfunction Card() {}\nexport default Card')
    const session = await sessionWith({
      ActionNode: entry('ActionNode', join(real, 'Card.tsx')),
      Card: entry('Card', join(real, 'Card.tsx')),
    })
    expect(await session.resolveExport(join(real, 'Card.tsx'), 'default')).toBe('Card')
  })

  it('answers an export under another name with the component the module renames', async () => {
    writeFileSync(join(real, 'Tag.tsx'), 'function Inner() {}\nfunction Other() {}\nexport { Inner as Tag, Other }')
    const session = await sessionWith({
      Inner: entry('Inner', join(real, 'Tag.tsx')),
      Other: entry('Other', join(real, 'Tag.tsx')),
    })
    expect(await session.resolveExport(join(real, 'Tag.tsx'), 'Tag')).toBe('Inner')
    expect(await session.resolveExport(join(real, 'Tag.tsx'), 'Other')).toBe('Other')
  })

  it('answers a default import by the name the story gave it only when the source does not say', async () => {
    writeFileSync(join(real, 'Panel.tsx'), 'export default makePanel()')
    const session = await sessionWith({
      Header: entry('Header', join(real, 'Panel.tsx')),
      Panel: entry('Panel', join(real, 'Panel.tsx')),
    })
    expect(await session.resolveExport(join(real, 'Panel.tsx'), 'default', 'Panel')).toBe('Panel')
    expect(await session.resolveExport(join(real, 'Panel.tsx'), 'default', 'Footer')).toBeUndefined()
    expect(await session.resolveExport(join(real, 'Panel.tsx'), 'default')).toBeUndefined()
  })

  it("answers a declared export with the file's only component whose display name differs", async () => {
    writeFileSync(
      join(real, 'Thumb.tsx'),
      "export function Thumb() {}\nThumb.displayName = '@mantine/core/ColorPickerThumb'\nexport function helper() {}\nconst Hidden = 1",
    )
    const session = await sessionWith({
      '@mantine/core/ColorPickerThumb': entry('@mantine/core/ColorPickerThumb', join(real, 'Thumb.tsx')),
    })
    expect(await session.resolveExport(join(real, 'Thumb.tsx'), 'Thumb')).toBe('@mantine/core/ColorPickerThumb')
    expect(await session.resolveExport(join(real, 'Thumb.tsx'), 'Hidden')).toBeUndefined()
    expect(await session.resolveExport(join(real, 'Thumb.tsx'), 'Missing')).toBeUndefined()
  })

  it('does not guess among several components when the export names none of their display names', async () => {
    writeFileSync(join(real, 'Pair.tsx'), 'export function Left() {}\nexport function Right() {}')
    const session = await sessionWith({
      'ui/Left': entry('ui/Left', join(real, 'Pair.tsx')),
      'ui/Right': entry('ui/Right', join(real, 'Pair.tsx')),
    })
    expect(await session.resolveExport(join(real, 'Pair.tsx'), 'Left')).toBeUndefined()
  })

  it('lists the components of a file, to say what a failed lookup chose from', async () => {
    const session = await sessionWith({
      A: entry('A', join(src, 'List.tsx')),
      B: entry('B', join(src, 'List.tsx')),
      C: entry('C', join(src, 'Other.tsx')),
    })
    expect(session.namesIn(join(src, 'List.tsx'))).toEqual(['A', 'B'])
    expect(session.namesIn(join(src, 'Missing.tsx'))).toEqual([])
  })
})

describe('DocgenSession.keyOfMember', () => {
  it('finds a sub-component by its namespace, the one beside the import when several share the name', async () => {
    const session = new DocgenSession(
      { srcDirs: [src] },
      new FakeBinding({
        components: {
          'Select.Option (select)': entry('Select.Option', join(src, 'select', 'Option.tsx')),
          'Select.Option (auto)': entry('Select.Option', join(src, 'auto', 'Option.tsx')),
          'Input.Password': entry('Input.Password', join(src, 'input', 'Password.tsx')),
        },
      }),
    )
    await session.fresh()
    expect(session.keyOfMember(['Input'], 'Password', join(src, 'input', 'index.tsx'))).toBe('Input.Password')
    expect(session.keyOfMember(['default', 'Select'], 'Option', join(src, 'select', 'index.tsx'))).toBe(
      'Select.Option (select)',
    )
    expect(session.keyOfMember(['Select'], 'Option', join(src, 'index.tsx'))).toBeUndefined()
    expect(session.keyOfMember(['Select'], 'Group', join(src, 'select', 'index.tsx'))).toBeUndefined()
  })
})

describe('DocgenSession.degraded', () => {
  it('counts warnings and errors that affect the key, not info, and forgets removed ones', async () => {
    const binding = new FakeBinding({
      components: { Button: entry('Button', join(src, 'Button.tsx')) },
      diagnostics: [warning(['Button']), warning(['Card'], 'info')],
    })
    const session = new DocgenSession({ srcDirs: [src], lazy: false }, binding)
    await session.fresh()
    expect(session.degraded('Button')).toBe(true)
    expect(session.degraded('Card')).toBe(false)

    binding.updates.push({ removedDiagnostics: [warning(['Button'])] })
    await session.fresh()
    expect(session.degraded('Button')).toBe(false)
  })

  it('lists each message once, however many diagnostics carry it', async () => {
    const binding = new FakeBinding({ diagnostics: [warning(['Button']), { ...warning(['Button']), line: 3 }] })
    const session = new DocgenSession({ srcDirs: [src], lazy: false }, binding)
    await session.fresh()
    expect(session.degradedBy('Button')).toEqual(['Cannot resolve type'])
  })
})

describe('DocgenSession.fresh', () => {
  it('initializes once and looks at the files on every call', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src], lazy: false }, binding)
    await session.fresh()
    await session.fresh()
    expect(binding.initializations).toBe(1)
    expect(binding.looks).toBe(2)
  })

  it('applies a deleted file, and the update removes its components', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src], lazy: false }, binding)
    await session.fresh()
    binding.updates.push({ removedKeys: ['Button'] })
    await session.fresh()
    expect(session.keyOf(join(src, 'Button.tsx'), 'Button')).toBeUndefined()
  })

  it('publishes a renamed component under its new key, and finds it by file and name afterwards', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src], lazy: false }, binding)
    await session.fresh()
    expect(session.keyOf(join(src, 'Button.tsx'), 'Button')).toBe('Button')
    binding.updates.push({
      removedKeys: ['Button'],
      updatedKeys: ['PrimaryButton'],
      updatedComponents: [entry('PrimaryButton', join(src, 'Button.tsx'))],
    })
    await session.fresh()
    expect(session.keyOf(join(src, 'Button.tsx'), 'PrimaryButton')).toBe('PrimaryButton')
    expect(session.keyOf(join(src, 'Button.tsx'), 'Button')).toBeUndefined()
  })

  it('ignores an update that is not newer than the one applied', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src], lazy: false }, binding)
    await session.fresh()
    binding.updates.push({ removedKeys: ['Button'] })
    await session.fresh()
    const stale = {
      schemaVersion: SUPPORTED_SCHEMA_VERSION,
      revision: 2,
      updatedKeys: ['Button'],
      updatedComponents: [entry('Button', join(src, 'Button.tsx'))],
    }
    binding.ensureFresh = async () =>
      JSON.stringify({ ...stale, removedKeys: [], removedDiagnostics: [], diagnostics: [] })
    await session.fresh()
    expect(session.keyOf(join(src, 'Button.tsx'), 'Button')).toBeUndefined()
  })

  it('shares one look among concurrent callers, and a caller during a look waits for a later one', async () => {
    const binding = new FakeBinding({ components: {} })
    const session = new DocgenSession({ srcDirs: [src], lazy: false }, binding)
    await session.fresh()
    const before = binding.looks

    let later: Promise<void>[] = []
    // Saved while the first look is under way: only a later look can see it.
    binding.duringLook = () => {
      later = [session.fresh(), session.fresh()]
    }
    await session.fresh()
    await Promise.all(later)
    expect(later).toHaveLength(2)
    expect(binding.looks - before).toBe(2)
  })

  it('passes the extraction options to the binding', async () => {
    const fake = new FakeBinding({})
    const seen: unknown[] = []
    fake.createSession = ((options: unknown) => (seen.push(options), 7)) as never
    const session = new DocgenSession(
      { srcDirs: [src], skipHtmlProps: true, loadDependencies: true, collapseInheritedProps: 'none' },
      fake,
    )
    await session.fresh()
    expect(seen[0]).toMatchObject({ htmlAttributes: 'none', loadDependencies: true, collapseInheritedProps: 'none' })
    const full = new DocgenSession({ srcDirs: [src], skipHtmlProps: true, htmlAttributes: 'full' }, fake)
    await full.fresh()
    expect(seen[1]).toMatchObject({ htmlAttributes: 'full' })
  })

  it('turns the on-disk caches off when told to, and leaves them to the binding otherwise', async () => {
    const fake = new FakeBinding({})
    const seen: unknown[] = []
    fake.createSession = ((options: unknown) => (seen.push(options), 7)) as never
    await new DocgenSession({ srcDirs: [src], cache: false }, fake).fresh()
    await new DocgenSession({ srcDirs: [src] }, fake).fresh()
    expect(seen).toMatchObject([{ cache: false }, { cache: undefined }])
  })
})

describe('DocgenSession, lazy', () => {
  it('asks for lazy resolution by default, and not when told otherwise', async () => {
    const fake = new FakeBinding({})
    const seen: unknown[] = []
    fake.createSession = ((options: unknown) => (seen.push(options), 7)) as never
    await new DocgenSession({ srcDirs: [src] }, fake).fresh()
    await new DocgenSession({ srcDirs: [src], lazy: false }, fake).fresh()
    expect(seen).toMatchObject([{ lazy: true }, { lazy: false }])
  })

  it('resolves a component when documentation asks, and says why its props may be incomplete', async () => {
    const binding = new FakeBinding({
      components: { Button: entry('Button', join(src, 'Button.tsx')), Card: entry('Card', join(src, 'Card.tsx')) },
      diagnostics: [warning(['Button']), warning(['Card'], 'info')],
    })
    binding.docs = { Button: rdtDoc('Button', ['label']), Card: rdtDoc('Card', ['title']) }
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    expect(binding.resolved).toEqual([])

    expect(await session.documentation('Button')).toEqual({
      doc: rdtDoc('Button', ['label']),
      degradedBy: ['Cannot resolve type'],
    })
    expect(await session.documentation('Card')).toEqual({ doc: rdtDoc('Card', ['title']), degradedBy: [] })
    expect(await session.documentation('Missing')).toEqual({ doc: undefined, degradedBy: [] })
    expect(binding.resolved).toEqual(['Button', 'Card', 'Missing'])
  })

  it('warms a lazy session up in the background, unless told not to or eager', async () => {
    const warmed = async (options: Partial<ConstructorParameters<typeof DocgenSession>[0]>) => {
      const binding = new FakeBinding({})
      await new DocgenSession({ srcDirs: [src], ...options }, binding).fresh()
      return binding.warmUps
    }
    expect(await warmed({ warmUp: true })).toEqual([8])
    expect(await warmed({})).toEqual([])
    expect(await warmed({ warmUp: true, lazy: false })).toEqual([])
  })

  it('resolves the rest in one pass once enough distinct components were asked for, and only once', async () => {
    const names = ['A', 'B', 'C', 'D', 'E']
    const binding = new FakeBinding({
      components: Object.fromEntries(names.map((name) => [name, entry(name, join(src, `${name}.tsx`))])),
    })
    binding.docs = Object.fromEntries(names.map((name) => [name, rdtDoc(name)]))
    const session = new DocgenSession({ srcDirs: [src], fillAfter: 3 }, binding)
    await session.fresh()
    for (const name of ['A', 'A', 'B']) await session.documentation(name)
    expect(binding.warmUps).toEqual([])
    await session.documentation('C')
    await session.documentation('D')
    expect(binding.warmUps).toEqual([2 ** 32 - 1])

    const never = new DocgenSession({ srcDirs: [src], fillAfter: 0 }, binding)
    await never.fresh()
    for (const name of names) await never.documentation(name)
    expect(binding.warmUps).toEqual([2 ** 32 - 1])
  })

  it('never resolves the rest in one pass by default, however many components were asked for', async () => {
    const names = Array.from({ length: 40 }, (_, i) => `C${i}`)
    const binding = new FakeBinding({
      components: Object.fromEntries(names.map((name) => [name, entry(name, join(src, `${name}.tsx`))])),
    })
    binding.docs = Object.fromEntries(names.map((name) => [name, rdtDoc(name)]))
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    for (const name of names) await session.documentation(name)
    expect(binding.warmUps).toEqual([])
  })

  it('forgets a listed key that has no component, so a lookup names what the file really has', async () => {
    const binding = new FakeBinding({
      components: {
        Card: entry('Card', join(src, 'Card.tsx')),
        CardContext: entry('CardContext', join(src, 'Card.tsx')),
      },
    })
    binding.docs = { Card: rdtDoc('Card') }
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    expect(session.namesIn(join(src, 'Card.tsx'))).toEqual(['Card', 'CardContext'])
    expect((await session.documentation('CardContext')).doc).toBeUndefined()
    expect(session.namesIn(join(src, 'Card.tsx'))).toEqual(['Card'])
  })

  it('looks again at where components are after an update that changes keys, and only then', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    expect(binding.initializations).toBe(1)

    await session.fresh()
    expect(binding.initializations).toBe(1)

    binding.output = { components: { PrimaryButton: entry('PrimaryButton', join(src, 'Button.tsx')) } }
    binding.updates.push({ removedKeys: ['Button'], updatedKeys: ['PrimaryButton'] })
    await session.fresh()
    expect(binding.initializations).toBe(2)
    expect(session.keyOf(join(src, 'Button.tsx'), 'PrimaryButton')).toBe('PrimaryButton')
    expect(session.keyOf(join(src, 'Button.tsx'), 'Button')).toBeUndefined()

    binding.updates.push({})
    await session.fresh()
    expect(binding.initializations).toBe(2)
  })

  it('answers an eager session from the index it holds, resolving nothing per request', async () => {
    const binding = new FakeBinding({
      components: { Button: entry('Button', join(src, 'Button.tsx')) },
      diagnostics: [warning(['Button'])],
    })
    binding.docs = { Button: rdtDoc('Button', ['label']) }
    const session = new DocgenSession({ srcDirs: [src], lazy: false }, binding)
    await session.fresh()
    expect(await session.documentation('Button')).toEqual({
      doc: rdtDoc('Button', ['label']),
      degradedBy: ['Cannot resolve type'],
    })
    expect(binding.resolved).toEqual([])
  })
})
