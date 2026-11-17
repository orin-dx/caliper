// construct: Readonly<T>
interface Base { a: string; b: number[] }
interface Props { p?: Readonly<Base> }
export function Row(props: Props) { return null }
