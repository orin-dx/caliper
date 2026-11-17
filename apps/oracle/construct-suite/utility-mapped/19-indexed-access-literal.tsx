// construct: T['key']
interface Base { a: string; b: { x: number } }
interface Props { p?: Base['b'] }
export function Row(props: Props) { return null }
