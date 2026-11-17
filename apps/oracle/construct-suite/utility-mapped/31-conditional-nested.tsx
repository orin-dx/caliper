// construct: nested conditional type
type Kind<T> = T extends string ? 'text' : T extends number ? 'numeric' : 'other'
interface Props { p?: Kind<number> }
export function Row(props: Props) { return null }
