// construct: mapped type -readonly -?
interface Base { readonly a?: string; readonly b?: number }
type Mutable<T> = { -readonly [K in keyof T]-?: T[K] }
interface Props { p?: Mutable<Base> }
export function Row(props: Props) { return null }
