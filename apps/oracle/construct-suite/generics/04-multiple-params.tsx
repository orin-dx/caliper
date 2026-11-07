// construct: multiple type parameters
interface Props<K extends string, V> {
  p?: Record<K, V>
}

export function Row<K extends string, V>(props: Props<K, V>) {
  return null
}
