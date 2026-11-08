// construct: Pick<T, K>
interface Base { a: string; b: number; c: boolean }
interface Props { p?: Pick<Base, 'a' | 'c'> }
export function Row(props: Props) { return null }
