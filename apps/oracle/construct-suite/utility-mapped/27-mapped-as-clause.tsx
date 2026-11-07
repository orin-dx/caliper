// construct: mapped type with as-clause (getters)
interface Base { name: string; age: number }
type Getters<T> = { [K in keyof T & string as `get${Capitalize<K>}`]: () => T[K] }
interface Props { p?: Getters<Base> }
export function Row(props: Props) { return null }
