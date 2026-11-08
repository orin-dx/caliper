// construct: homomorphic mapped type (keyof T)
interface Base { a?: string; readonly b: number }
type Wrapped<T> = { [K in keyof T]: T[K][] }
interface Props { p?: Wrapped<Base> }
export function Row(props: Props) { return null }
