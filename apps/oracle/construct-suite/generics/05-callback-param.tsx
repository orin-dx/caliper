// construct: type parameter in callback parameter
interface Props<T> {
  p?: (value: T) => void
}

export function Row<T>(props: Props<T>) {
  return null
}
