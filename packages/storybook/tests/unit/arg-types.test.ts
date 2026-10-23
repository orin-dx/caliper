import { describe, expect, it } from 'vitest'
import type { RdtComponentDoc, RdtProp } from 'caliper-docgen-napi'
import { loadExtractArgTypes, toArgTypes } from '../../src/arg-types.js'

function prop(name: string, fields: Partial<RdtProp> = {}): RdtProp {
  return {
    name,
    type: { name: 'string' },
    required: false,
    defaultValue: null,
    description: '',
    parent: null,
    ...fields,
  }
}

function doc(...props: RdtProp[]): RdtComponentDoc {
  return {
    displayName: 'Button',
    description: '',
    props: Object.fromEntries(props.map((p) => [p.name, p])),
    methods: [],
    tags: {},
    composes: [],
  }
}

describe('toArgTypes', () => {
  it('turns a literal union into an enum control with its values joined in the table', async () => {
    const variant = prop('variant', {
      description: 'Visual weight.',
      type: { name: 'enum', raw: '"primary" | "ghost"', value: [{ value: '"primary"' }, { value: '"ghost"' }] },
      defaultValue: { value: '"primary"', computed: false },
    })
    expect(toArgTypes(doc(variant), await loadExtractArgTypes())).toEqual({
      variant: {
        name: 'variant',
        description: 'Visual weight.',
        type: { required: false, name: 'enum', raw: '"primary" | "ghost"', value: ['primary', 'ghost'] },
        table: {
          type: { summary: '"primary" | "ghost"' },
          defaultValue: { summary: '"primary"' },
          jsDocTags: undefined,
        },
      },
    })
  })

  it('drops React attribute props declared in node_modules and keeps a local interface of the same name', async () => {
    const ref = prop('ref', { parent: { name: 'RefAttributes', fileName: 'node_modules/@types/react/index.d.ts' } })
    const key = prop('key', { parent: { name: 'Attributes', fileName: '/app/node_modules/@types/react/index.d.ts' } })
    const local = prop('tone', { parent: { name: 'Attributes', fileName: 'src/Attributes.ts' } })
    expect(Object.keys(toArgTypes(doc(ref, key, local), await loadExtractArgTypes()) ?? {})).toEqual(['tone'])
  })

  it('shows a computed default and prettifies an object default the way React does', async () => {
    const size = prop('size', { defaultValue: { value: 'SIZES.md', computed: true } })
    const meta = prop('meta', {
      type: { name: '{ author: string }' },
      defaultValue: { value: "{author: 'anon'}", computed: false },
    })
    const argTypes = toArgTypes(doc(size, meta), await loadExtractArgTypes())
    expect(argTypes?.size?.table?.defaultValue).toEqual({ summary: 'SIZES.md' })
    expect(argTypes?.meta?.table?.defaultValue).toEqual({ summary: "{ author: 'anon' }" })
  })

  it('maps a required function prop to a function control', async () => {
    const onClick = prop('onClick', { required: true, type: { name: '(event: MouseEvent) => void' } })
    expect(toArgTypes(doc(onClick), await loadExtractArgTypes())?.onClick?.type).toEqual({
      required: true,
      name: 'function',
      value: '(event: MouseEvent) => void',
    })
  })
})
