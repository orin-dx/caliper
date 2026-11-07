// construct: Promise<T[]> from type parameter
interface Props<T> {
  p?: () => Promise<T[]>
}

export function Row<T>(props: Props<T>) {
  return null
}
