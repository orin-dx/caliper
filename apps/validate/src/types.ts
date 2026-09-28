/** Normalized prop representation — common subset of rdg + rdt output */
export interface NormalizedProp {
  name: string
  required: boolean
  /** Verbatim source text, not normalized — compared as-is across tools. */
  type: string
  description: string
  defaultValue?: string
  /** First declaring interface/type-alias name (semantic tool only). */
  parent?: string
  /** Every declaring interface/type-alias name (semantic tool only). */
  declarations?: string[]
}

export interface NormalizedComponent {
  displayName: string
  description: string
  props: Record<string, NormalizedProp>
}

export type NormalizedOutput = Record<string, NormalizedComponent>

export interface ToolResult {
  tool: 'react-docgen' | 'react-docgen-typescript' | 'oxc-react-docgen' | 'semantic'
  fixture: string
  durationMs: number
  output: NormalizedOutput
  /** HTML element(s) the component inherits from (our tool only) */
  inheritedElements?: string[]
  /** Notable inherited prop names surfaced from the HTML element (our tool only) */
  notableInheritedNames?: string[]
  error?: string
}
