// The rule that sorts a collapsed layer's props into ambient and API is checkable: it reads the layer's file and type and
// the prop's name, so each clause is tested alone, and the first clause that fits decides.
import { describe, expect, it } from 'vitest'
import { classify, declaredMemberType, GLOBAL_ATTRIBUTES, layerOnly, topApi } from '../src/layer-only.ts'

const DOM = '/r/node_modules/@react-types/shared/src/dom.d.ts'
const DEP = '/r/node_modules/dep/index.d.ts'
const item = (layer: string, layerFile: string, prop: string) => ({ layer, layerFile, prop })

describe('classify', () => {
  it('calls a prop ambient when its layer is a DOM file', () => {
    expect.hasAssertions()
    expect(classify(item('GlobalDOMEvents', DOM, 'onCopy'))).toStrictEqual({ kind: 'ambient', why: 'dom file' })
    expect(classify(item('Anything', '/r/node_modules/@types/react/index.d.ts', 'onClick'))).toStrictEqual({
      kind: 'ambient',
      why: 'dom file',
    })
  })

  it('calls a prop ambient when its layer is a DOM family by name, wherever it is declared', () => {
    expect.hasAssertions()
    for (const layer of [
      'DOMAttributes',
      'AriaAttributes',
      'AllHTMLAttributes',
      'SVGAttributes',
      'ButtonHTMLAttributes',
      'AriaButtonAttributes',
    ]) {
      expect(classify(item(layer, DEP, 'onClick'))).toStrictEqual({ kind: 'ambient', why: 'dom type' })
    }
  })

  it('calls a prop ambient when its layer is a style system', () => {
    expect.hasAssertions()
    for (const layer of ['StyleProps', 'MantineStyleProps', 'SystemProps', 'SpaceProps', 'LayoutProps']) {
      expect(classify(item(layer, DEP, 'margin'))).toStrictEqual({ kind: 'ambient', why: 'style type' })
    }
  })

  it('calls a global attribute ambient in any layer, but not an element-specific name', () => {
    expect.hasAssertions()
    for (const prop of ['className', 'style', 'id', 'tabIndex', 'aria-label', 'data-testid']) {
      expect(classify(item('DepProps', DEP, prop))).toStrictEqual({ kind: 'ambient', why: 'global attribute' })
    }
    for (const prop of ['value', 'defaultValue', 'type', 'name', 'disabled', 'children', 'onChange']) {
      expect(classify(item('DepProps', DEP, prop))).toStrictEqual({ kind: 'api', why: 'dependency api' })
    }
    expect(GLOBAL_ATTRIBUTES.has('defaultValue')).toBe(false)
  })

  it('calls anything else a dependency layer names API', () => {
    expect.hasAssertions()
    expect(classify(item('ComboboxRootProps', DEP, 'asChild'))).toStrictEqual({ kind: 'api', why: 'dependency api' })
  })

  it('lets the first clause that fits decide', () => {
    expect.hasAssertions()
    expect(classify(item('DepProps', DOM, 'value')).why).toBe('dom file')
    expect(classify(item('StyleProps', DEP, 'className')).why).toBe('style type')
  })
})

describe('layerOnly', () => {
  const caliper = {
    components: {
      A: {
        displayName: 'A',
        props: { own: 'p' },
        // A prop a notable-inherited map already lists is not layer-only.
        notableInherited: { listed: 'p' },
        inheritance: [
          { typeName: 'GlobalDOMEvents', fileName: DOM, totalProps: 64, members: 'n1' },
          { typeName: 'DepProps', fileName: DEP, totalProps: 35, members: ['value', 'own', 'listed', 'onCopy'] },
          { typeName: 'ButtonHTMLAttributes', fileName: '/r/node_modules/@types/react/index.d.ts', totalProps: 200 },
        ],
      },
    },
    nameLists: { n1: ['onCopy', 'onCut'] },
    props: { p: {} },
  }

  it('names the props a collapsed layer lists that the component does not, once each', () => {
    expect.hasAssertions()
    expect(layerOnly(caliper).map((i) => `${i.layer}.${i.prop}`)).toStrictEqual([
      'GlobalDOMEvents.onCopy',
      'GlobalDOMEvents.onCut',
      'DepProps.value',
    ])
  })
})

describe('declaredMemberType', () => {
  const text = `export interface DepProps { value?: string | number; mode: 'a' | 'b'; onChange(v: string): void }
export type Alias = { size: number }`

  it('reads the type a declaration writes for one member, on one line', () => {
    expect.hasAssertions()
    expect(declaredMemberType(text, 'DepProps', 'value')).toBe('string | number')
    expect(declaredMemberType(text, 'DepProps', 'mode')).toBe("'a' | 'b'")
    expect(declaredMemberType(text, 'Alias', 'size')).toBe('number')
  })

  it('has nothing for a method, a missing member or a missing type', () => {
    expect.hasAssertions()
    expect(declaredMemberType(text, 'DepProps', 'onChange')).toBeUndefined()
    expect(declaredMemberType(text, 'DepProps', 'nope')).toBeUndefined()
    expect(declaredMemberType(text, 'Nope', 'value')).toBeUndefined()
  })
})

describe('topApi', () => {
  const items = [
    ...['A', 'B', 'C'].map((key) => ({
      key,
      component: key,
      layer: 'DepProps',
      layerFile: DEP,
      layerSize: 40,
      prop: 'zebra',
    })),
    { key: 'A', component: 'A', layer: 'DepProps', layerFile: DEP, layerSize: 40, prop: 'value' },
    { key: 'A', component: 'A', layer: 'DepProps', layerFile: DEP, layerSize: 40, prop: 'asChild' },
    { key: 'A', component: 'A', layer: 'GlobalDOMEvents', layerFile: DOM, layerSize: 64, prop: 'onCopy' },
  ]

  it('leads with the props a reader needs, then the breadth, and leaves ambient props out', () => {
    expect.hasAssertions()
    const top = topApi(items, 10, () => 'export interface DepProps { value: string }')
    expect(top.map((t) => `${t.category}:${t.prop}:${t.components}`)).toStrictEqual([
      'state:value:1',
      'composition:asChild:1',
      'other:zebra:3',
    ])
    expect(top[0]).toMatchObject({ layer: 'DepProps', declared: 'string', file: 'dep/index.d.ts' })
    expect(top[1].declared).toBeUndefined()
  })

  it('keeps at most `limit`', () => {
    expect.hasAssertions()
    expect(topApi(items, 1, () => undefined)).toHaveLength(1)
  })
})
