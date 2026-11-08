// construct: mapped type as-clause filtering keys to never
interface Base { a: string; b: number; c: string }
type OnlyStrings<T> = { [K in keyof T as T[K] extends string ? K : never]: T[K] }
interface Props { p?: OnlyStrings<Base> }
export function Row(props: Props) { return null }
