import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import { DocgenSession } from '../../src/session.js'
import { FakeBinding, entry, warning } from './fake-binding.js'

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
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    expect(session.degraded('Button')).toBe(true)
    expect(session.degraded('Card')).toBe(false)

    binding.updates.push({ removedDiagnostics: [warning(['Button'])] })
    await session.fresh()
    expect(session.degraded('Button')).toBe(false)
  })

  it('lists each message once, however many diagnostics carry it', async () => {
    const binding = new FakeBinding({ diagnostics: [warning(['Button']), { ...warning(['Button']), line: 3 }] })
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    expect(session.degradedBy('Button')).toEqual(['Cannot resolve type'])
  })
})

describe('DocgenSession.fresh', () => {
  it('initializes once and looks at the files on every call', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    await session.fresh()
    expect(binding.initializations).toBe(1)
    expect(binding.looks).toBe(2)
  })

  it('applies a deleted file, and the update removes its components', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    binding.updates.push({ removedKeys: ['Button'] })
    await session.fresh()
    expect(session.keyOf(join(src, 'Button.tsx'), 'Button')).toBeUndefined()
  })

  it('publishes a renamed component under its new key, and finds it by file and name afterwards', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src] }, binding)
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
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    binding.updates.push({ removedKeys: ['Button'] })
    await session.fresh()
    const stale = {
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
    const session = new DocgenSession({ srcDirs: [src] }, binding)
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
})
