// Props that reach a reader only as a name: a collapsed inheritance layer lists the names it declares (`members`, an
// interned name list in the canonical JSON) and the component's `props` does not hold them. This sorts those props into
// AMBIENT (what every element has, or a style system: a props table would not lead with them) and API (what a
// dependency's component exposes: its modes, state and composition), by a rule that reads only the layer's type name,
// its file and the prop's name. It describes the default collapse (`dependencies`) of the CLI, the NAPI binding and the
// Vite plugin; the Storybook addon runs at `none` and collapses nothing. It measures; it does not change the resolver.
// Pure apart from `declaredMemberType`, which reads one declaration file.
import { readFileSync } from 'node:fs'
import ts from 'typescript'
import { layerMembers, listedProps } from './canonical.ts'

/** One prop a collapsed layer names and the component does not list. */
export interface LayerOnly {
  /** The component's key in the canonical JSON. */
  key: string
  component: string
  /** The layer's declaring type and file, and how many props it declares. */
  layer: string
  layerFile: string
  layerSize: number
  prop: string
}

export type Kind = 'api' | 'ambient'
/** Which clause of the rule decided: the layer's file, the layer's type, a style system, a global attribute, or, when none
 * of those fits, the prop is a dependency's API. */
export type Why = 'dom file' | 'dom type' | 'style type' | 'global attribute' | 'dependency api'

/** The attributes every element has, which no component's API is about. `defaultValue`, `value`, `type`, `name` and the
 * like are NOT here: they are element-specific, and a dependency's component often uses them as its own API. */
export const GLOBAL_ATTRIBUTES = new Set([
  'className',
  'style',
  'id',
  'title',
  'lang',
  'dir',
  'hidden',
  'tabIndex',
  'role',
  'slot',
  'accessKey',
  'draggable',
  'contentEditable',
  'spellCheck',
  'translate',
  'inputMode',
  'nonce',
])

const DOM_FILE = /\/@types\/react\/|\/@react-types\/shared\/src\/dom\.d\.[cm]?ts$/
const DOM_TYPE =
  /^(GlobalDOMEvents|DOMAttributes|AriaAttributes|AllHTMLAttributes|SVGAttributes|\w*HTMLAttributes|Aria\w*Attributes)$/
const STYLE_TYPE =
  /^(\w*StyleProps?|SystemProps|(Space|Layout|Color|Typography|Border|Flex|Grid|Position|Shadow|Background)Props)$/

/** Every prop a collapsed layer names that the component does not list, once per component and prop. */
export function layerOnly(caliper: any): LayerOnly[] {
  const out: LayerOnly[] = []
  for (const [key, component] of Object.entries<any>(caliper.components)) {
    const listed = listedProps(caliper, component)
    const seen = new Set<string>()
    for (const layer of component.inheritance ?? []) {
      for (const prop of layerMembers(caliper, layer)) {
        if (prop in listed || seen.has(prop)) continue
        seen.add(prop)
        out.push({
          key,
          component: component.displayName,
          layer: layer.typeName,
          layerFile: String(layer.fileName),
          layerSize: layer.totalProps,
          prop,
        })
      }
    }
  }
  return out
}

/** The rule. The first clause that fits decides; a prop none of them fits is API.
 *
 *   1. dom file          the layer's file is `@types/react`, or react-aria's `@react-types/shared/src/dom.d.ts`
 *   2. dom type          the layer's type is a DOM family: `GlobalDOMEvents`, `DOMAttributes`, `AriaAttributes`,
 *                        `AllHTMLAttributes`, `SVGAttributes`, `*HTMLAttributes`, `Aria*Attributes`
 *   3. style type        the layer's type is a style system: `*StyleProps`, `SystemProps`, `Space|Layout|Color|…Props`
 *   4. global attribute  the prop is one every element has (`GLOBAL_ATTRIBUTES`), or starts `aria-` or `data-`
 *   5. dependency api    anything else a collapsed layer names
 */
export function classify(item: Pick<LayerOnly, 'layer' | 'layerFile' | 'prop'>): { kind: Kind; why: Why } {
  if (DOM_FILE.test(item.layerFile)) return { kind: 'ambient', why: 'dom file' }
  if (DOM_TYPE.test(item.layer)) return { kind: 'ambient', why: 'dom type' }
  if (STYLE_TYPE.test(item.layer)) return { kind: 'ambient', why: 'style type' }
  if (GLOBAL_ATTRIBUTES.has(item.prop) || /^(aria|data)-/.test(item.prop))
    return { kind: 'ambient', why: 'global attribute' }
  return { kind: 'api', why: 'dependency api' }
}

/** Why an API prop matters to a reader, by what the prop is called. The first that fits; the rest are `other`. */
export const CATEGORIES: { name: string; match: RegExp; matters: string }[] = [
  {
    name: 'state',
    match:
      /^(value|defaultValue|checked|defaultChecked|open|defaultOpen|visible|selected\w*|expanded\w*|on(Value|Open|Checked|Selected|Expanded)?Change|onClose|onOpen|afterOpenChange)$/,
    matters: 'what the component holds or reports, so what a reader sets to drive it',
  },
  {
    name: 'mode',
    match: /^(mode|type|variant|size|multiple|orientation|collapsible|selectionMode|status|placement|side|align)$/,
    matters: 'chooses the form or behaviour of the component',
  },
  {
    name: 'composition',
    match: /^(asChild|render|as|components|slots|slotProps|container|portal\w*)$/,
    matters: 'changes the element or content the component renders',
  },
  {
    name: 'constraint',
    match: /^(disabled|required|readOnly|name|min|max|step|maxLength|loading|invalid)$/,
    matters: 'validation, disabled or loading state',
  },
  {
    name: 'data',
    match: /^(items|options|data|dataSource|columns|locale|format\w*|filter\w*)$/,
    matters: 'what the component shows',
  },
]
const categoryOf = (prop: string) => CATEGORIES.find((c) => c.match.test(prop))
/** Where a prop's category sits in the order; `other` is last. */
const order = (prop: string) => {
  const index = CATEGORIES.findIndex((c) => c.match.test(prop))
  return index === -1 ? CATEGORIES.length : index
}

/** The type a declaration file writes for `typeName.prop`, as text; `undefined` when it is not a plain member. */
export function declaredMemberType(text: string, typeName: string, prop: string): string | undefined {
  const source = ts.createSourceFile('layer.d.ts', text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  const typed = (members: ts.NodeArray<ts.TypeElement>) => {
    const found = members.find((m) => ts.isPropertySignature(m) && m.name.getText(source) === prop)
    return found && ts.isPropertySignature(found) ? found.type?.getText(source).replaceAll(/\s+/g, ' ') : undefined
  }
  for (const statement of source.statements) {
    if (ts.isInterfaceDeclaration(statement) && statement.name.text === typeName) return typed(statement.members)
    if (
      ts.isTypeAliasDeclaration(statement) &&
      statement.name.text === typeName &&
      ts.isTypeLiteralNode(statement.type)
    ) {
      return typed(statement.type.members)
    }
  }
  return undefined
}

export interface TopApi {
  prop: string
  layer: string
  /** The layer's file from `node_modules` on. */
  file: string
  components: number
  category: string
  matters: string
  declared?: string
}

const shortFile = (file: string) => file.replace(/^.*\/node_modules\/(\.pnpm\/[^/]+\/node_modules\/)?/, '')

/** The `limit` API props a reader most needs, by this rule: category first (state, mode, composition, constraint, data,
 * other), then the number of components it is layer-only in, then name. `read` supplies a declaration file's text. */
export function topApi(items: LayerOnly[], limit: number, read: (file: string) => string | undefined): TopApi[] {
  const groups = new Map<string, { item: LayerOnly; components: Set<string> }>()
  for (const item of items) {
    if (classify(item).kind !== 'api') continue
    const key = `${item.layer}\u0000${item.layerFile}\u0000${item.prop}`
    const group = groups.get(key) ?? { item, components: new Set() }
    group.components.add(item.key)
    groups.set(key, group)
  }
  return [...groups.values()]
    .toSorted(
      (a, b) =>
        order(a.item.prop) - order(b.item.prop) ||
        b.components.size - a.components.size ||
        a.item.prop.localeCompare(b.item.prop),
    )
    .slice(0, limit)
    .map(({ item, components }) => {
      const category = categoryOf(item.prop)
      const text = read(item.layerFile)
      return {
        prop: item.prop,
        layer: item.layer,
        file: shortFile(item.layerFile),
        components: components.size,
        category: category?.name ?? 'other',
        matters: category?.matters ?? 'a prop of the dependency component',
        declared: text === undefined ? undefined : declaredMemberType(text, item.layer, item.prop),
      }
    })
}

/** A declaration file's text, or `undefined` when it cannot be read. */
export function readFile(file: string): string | undefined {
  try {
    return readFileSync(file, 'utf8')
  } catch {
    return undefined
  }
}
