// The layer-only statistics on a corpus small enough to count by hand.
import { describe, expect, it } from 'vitest'
import { analyse, ambientListedToday, expandedDependencySources, layerCounts } from '../src/layer-only-stats.ts'

const DOM = '/r/node_modules/@react-types/shared/src/dom.d.ts'
const DEP = '/r/node_modules/dep/index.d.ts'
const SMALL = '/r/node_modules/dep/small.d.ts'
const REACT = '/r/node_modules/@types/react/index.d.ts'

// Two components. Both extend a 64-prop DOM-events layer and a 35-prop dependency layer, collapsed. B also lists a
// small dependency source and a small DOM source in full, and an `@types/react` layer.
const prop = (name: string, parent?: { name: string; fileName: string }) => ({
  name,
  type: { kind: 'string' },
  parent: parent ?? null,
})
const caliper = {
  components: {
    A: {
      displayName: 'A',
      props: { own: 'own' },
      inheritance: [
        { typeName: 'GlobalDOMEvents', fileName: DOM, totalProps: 64, members: 'events' },
        { typeName: 'DepProps', fileName: DEP, totalProps: 35, members: 'dep' },
      ],
    },
    B: {
      displayName: 'B',
      props: { own: 'own', size: 'size', onFocus: 'focus' },
      inheritance: [
        { typeName: 'GlobalDOMEvents', fileName: DOM, totalProps: 64, members: 'events' },
        { typeName: 'DepProps', fileName: DEP, totalProps: 35, members: 'dep' },
        { typeName: 'ButtonHTMLAttributes', fileName: REACT, totalProps: 200, htmlElement: 'button' },
      ],
    },
  },
  nameLists: { events: ['onCopy', 'onCut'], dep: ['value', 'mode', 'className'] },
  props: {
    own: prop('own'),
    size: prop('size', { name: 'SmallProps', fileName: SMALL }),
    focus: prop('onFocus', { name: 'FocusEvents', fileName: DOM }),
  },
}

// The same corpus with nothing collapsed: every layer-only prop is listed.
const expanded = {
  components: {
    A: {
      displayName: 'A',
      props: { own: 'own', onCopy: 'copy', onCut: 'cut', value: 'value', mode: 'mode', className: 'cls' },
    },
    B: {
      displayName: 'B',
      props: {
        own: 'own',
        size: 'size',
        onFocus: 'focus',
        onCopy: 'copy',
        onCut: 'cut',
        value: 'value',
        mode: 'mode',
        className: 'cls',
      },
    },
  },
  props: {
    ...caliper.props,
    copy: prop('onCopy'),
    cut: prop('onCut'),
    value: prop('value'),
    mode: prop('mode'),
    cls: prop('className'),
  },
}

describe('layerCounts', () => {
  const counts = layerCounts(caliper)

  it('counts each layer once, collapsed or an HTML-element layer, by size', () => {
    expect.hasAssertions()
    expect(counts.distinct_layers).toBe(3)
    expect(counts.html_layers).toBe(1)
    expect(counts.collapsed).toStrictEqual({
      layers: 2,
      above_threshold: 2,
      at_or_below_threshold: 0,
      by_size: { '31-40': 1, '61+': 1 },
      ambient_by_type_or_file: 1,
    })
  })

  it('counts the dependency sources the output lists in full, from the type that declares each prop', () => {
    expect.hasAssertions()
    expect(expandedDependencySources(caliper)).toStrictEqual([
      { layer: 'SmallProps', file: SMALL, size: 1 },
      { layer: 'FocusEvents', file: DOM, size: 1 },
    ])
    expect(counts.expanded_dependency).toStrictEqual({
      layers: 2,
      above_threshold: 0,
      at_or_below_threshold: 2,
      ambient_by_type_or_file: 1,
    })
  })
})

describe('analyse', () => {
  const report = analyse({ caliper, expanded, bytes: { default: 1000, expanded: 1400 } }, () => undefined)

  it('sorts the layer-only props, per component, into API and ambient with the clause that decided', () => {
    expect.hasAssertions()
    // A and B each: onCopy, onCut (dom file), value, mode (dependency api), className (global attribute).
    expect(report.layer_only).toStrictEqual({
      props: 10,
      api: 4,
      ambient: 6,
      by_clause: { 'dom file': 4, 'dependency api': 4, 'global attribute': 2 },
      distinct_api: 2,
      distinct_ambient: 3,
      components_with_api: 2,
    })
  })

  it('says how the size rule sits against relevance', () => {
    expect.hasAssertions()
    expect(report.size_rule_against_relevance).toStrictEqual({
      collapsed_layers_with_api_props: 1,
      collapsed_layers_all_ambient: 1,
      api_props_in_layers_of_31_to_40: 4,
    })
  })

  it('measures what listing the API props adds and what the whole expansion adds', () => {
    expect.hasAssertions()
    expect(report.effect?.api).toMatchObject({ props: 4, distinct_entries: 2 })
    expect(report.effect?.ambient).toMatchObject({ props: 6, distinct_entries: 3 })
    expect(report.effect?.all_expanded_bytes_delta).toBe(400)
    expect(report.effect?.default_bytes).toBe(1000)
  })

  it('measures the ambient props a small dependency source lists today', () => {
    expect.hasAssertions()
    // B lists onFocus from a DOM-file source; the SmallProps `size` is API and is not counted.
    expect(ambientListedToday(caliper)).toMatchObject({ props: 1, distinct_entries: 1 })
  })

  it('reports the fully expanded configuration beside the default one: nothing collapsed, so nothing layer-only', () => {
    expect.hasAssertions()
    // The Storybook addon runs at `none`, which lists every inherited prop. The default config lists 4 component props
    // (A: own; B: own, size, onFocus) and holds 10 more as names; the expanded one lists all 14.
    expect(report.expanded_config).toStrictEqual({
      collapsed_layers: 0,
      layer_only_props: 0,
      listed_props: { default: 4, expanded: 14 },
    })
  })

  it('has no effect and no expanded configuration without the expanded run', () => {
    expect.hasAssertions()
    const without = analyse({ caliper }, () => undefined)
    expect(without.effect).toBeUndefined()
    expect(without.expanded_config).toBeUndefined()
  })
})
