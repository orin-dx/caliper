// construct: mapped type +readonly +?
interface Base { a: string; b: number }
type Frozen<T> = { +readonly [K in keyof T]+?: T[K] }
interface Props { p?: Frozen<Base> }
export function Row(props: Props) { return null }
