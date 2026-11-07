// construct: T[keyof T] on empty interface (never)
interface Empty {}
interface Props { p?: Empty[keyof Empty] }
export function Row(props: Props) { return null }
