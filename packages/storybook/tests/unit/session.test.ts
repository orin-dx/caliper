import { mkdirSync, mkdtempSync, rmSync, utimesSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { DocgenSession } from '../../src/session.js'
import { FakeBinding, entry, warning } from './fake-binding.js'

let dir: string
let src: string

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'caliper-sb-session-'))
  src = join(dir, 'src')
  mkdirSync(join(src, 'node_modules'), { recursive: true })
  writeFileSync(join(src, 'Button.tsx'), 'export function Button() {}')
  writeFileSync(join(src, 'node_modules', 'Dep.tsx'), 'export function Dep() {}')
})

afterEach(() => rmSync(dir, { recursive: true, force: true }))

/** Moves a file's mtime forward, so a rewrite within the same millisecond still counts as a change. */
function touch(path: string, content: string, secondsAhead = 10): void {
  writeFileSync(path, content)
  const later = new Date(Date.now() + secondsAhead * 1000)
  utimesSync(path, later, later)
}

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

    binding.updates.set(join(src, 'Button.tsx'), { removedDiagnostics: [warning(['Button'])] })
    touch(join(src, 'Button.tsx'), 'export function Button(props: { a: string }) {}')
    await session.fresh()
    expect(session.degraded('Button')).toBe(false)
  })
})

describe('DocgenSession.fresh', () => {
  it('initializes once and applies only the source files saved since the last look', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    await session.fresh()
    expect(binding.initializations).toBe(1)
    expect(binding.changed).toEqual([])

    touch(join(src, 'Button.tsx'), 'export function Button(props: { a: string }) {}')
    writeFileSync(join(src, 'Card.tsx'), 'export function Card() {}')
    writeFileSync(join(src, 'notes.md'), '# not source')
    touch(join(src, 'node_modules', 'Dep.tsx'), 'export function Dep(props: { a: string }) {}')
    await session.fresh()
    expect(binding.changed.toSorted()).toEqual([join(src, 'Button.tsx'), join(src, 'Card.tsx')])
  })

  it('applies a deleted file, and the update removes its components', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    binding.updates.set(join(src, 'Button.tsx'), { removedKeys: ['Button'] })
    rmSync(join(src, 'Button.tsx'))
    await session.fresh()
    expect(binding.changed).toEqual([join(src, 'Button.tsx')])
    expect(session.keyOf(join(src, 'Button.tsx'), 'Button')).toBeUndefined()
  })

  it('publishes a renamed component under its new key', async () => {
    const binding = new FakeBinding({ components: { Button: entry('Button', join(src, 'Button.tsx')) } })
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()
    binding.updates.set(join(src, 'Button.tsx'), {
      removedKeys: ['Button'],
      updatedKeys: ['PrimaryButton'],
      updatedComponents: [entry('PrimaryButton', join(src, 'Button.tsx'))],
    })
    touch(join(src, 'Button.tsx'), 'export function PrimaryButton() {}')
    await session.fresh()
    expect(session.keyOf(join(src, 'Button.tsx'), 'PrimaryButton')).toBe('PrimaryButton')
    expect(session.keyOf(join(src, 'Button.tsx'), 'Button')).toBeUndefined()
  })

  it('shares one look among concurrent callers, and a caller during a look waits for a later one', async () => {
    const binding = new FakeBinding({ components: {} })
    const session = new DocgenSession({ srcDirs: [src] }, binding)
    await session.fresh()

    let later: Promise<void>[] = []
    binding.duringExtraction = () => {
      // Saved while the first look is applying its changes: only a later look can see it.
      touch(join(src, 'Button.tsx'), 'export function Button(props: { b: string }) {}', 20)
      later = [session.fresh(), session.fresh()]
    }
    touch(join(src, 'Button.tsx'), 'export function Button(props: { a: string }) {}')
    await session.fresh()
    await Promise.all(later)
    expect(later).toHaveLength(2)
    expect(binding.changed).toEqual([join(src, 'Button.tsx'), join(src, 'Button.tsx')])
  })
})
