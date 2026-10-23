// `__docgenInfo` (react-docgen-typescript's shape) to argTypes, the way `@storybook/react`'s own docgen worker does it
// (src/docgen/buildDocgen.ts): React's `Attributes`, `RefAttributes` and `ClassAttributes` props from node_modules are
// dropped, defaults are marked `computed: false`, and React's `extractArgTypes` converts the rest. Calling React's
// converter, not a copy of it, keeps controls, type summaries and default-value summaries identical to Storybook's.

import type { RdtComponentDoc, RdtProp } from 'caliper-docgen-napi'
import type { StrictArgTypes } from 'storybook/internal/types'

/** `parameters.docs.extractArgTypes` from `@storybook/react/entry-preview-argtypes`. */
export type ExtractArgTypes = (component: { __docgenInfo: RdtComponentDoc }) => StrictArgTypes | null

const REACT_ATTRIBUTE_INTERFACES = new Set(['Attributes', 'RefAttributes', 'ClassAttributes'])

function isReactAttribute(prop: RdtProp): boolean {
  return (
    prop.parent !== null &&
    REACT_ATTRIBUTE_INTERFACES.has(prop.parent.name) &&
    /(^|[\\/])node_modules[\\/]/.test(prop.parent.fileName)
  )
}

export function toArgTypes(doc: RdtComponentDoc, extractArgTypes: ExtractArgTypes): StrictArgTypes | undefined {
  const props = Object.fromEntries(
    Object.entries(doc.props)
      .filter(([, prop]) => !isReactAttribute(prop))
      .map(([name, prop]) => [
        name,
        prop.defaultValue ? { ...prop, defaultValue: { ...prop.defaultValue, computed: false } } : prop,
      ]),
  )
  return extractArgTypes({ __docgenInfo: { ...doc, props } }) ?? undefined
}

/** React's converter, loaded on first use: it is a preview entry, so only a docgen request should pay for it. */
export async function loadExtractArgTypes(): Promise<ExtractArgTypes> {
  const { parameters } = (await import('@storybook/react/entry-preview-argtypes')) as {
    parameters: { docs: { extractArgTypes: ExtractArgTypes } }
  }
  return parameters.docs.extractArgTypes
}
