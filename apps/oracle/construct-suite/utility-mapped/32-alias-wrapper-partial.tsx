// construct: type A = Partial<B>; p: A
interface B { a: string; b: number }
type A = Partial<B>
interface Props { p?: A }
export function Row(props: Props) { return null }
