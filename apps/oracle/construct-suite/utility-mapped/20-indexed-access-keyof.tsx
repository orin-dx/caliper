// construct: T[keyof T]
interface Base { a: string; b: number; c: boolean }
interface Props { p?: Base[keyof Base] }
export function Row(props: Props) { return null }
