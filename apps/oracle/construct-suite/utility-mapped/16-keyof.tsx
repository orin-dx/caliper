// construct: keyof T
interface Base { a: string; b: number; c: boolean }
interface Props { p?: keyof Base }
export function Row(props: Props) { return null }
