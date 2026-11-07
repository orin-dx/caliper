// construct: Partial<T>
interface Base { a: string; b: number }
interface Props { p?: Partial<Base> }
export function Row(props: Props) { return null }
