// construct: Omit<T, K>
interface Base { a: string; b: number; c: boolean }
interface Props { p?: Omit<Base, 'b'> }
export function Row(props: Props) { return null }
