// construct: template literal with Capitalize and string
type Ev = 'click' | 'focus'
interface Props { p?: `on${Capitalize<Ev>}` | `data-${string}` }
export function Row(props: Props) { return null }
