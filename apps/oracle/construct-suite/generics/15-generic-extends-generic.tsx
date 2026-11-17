// construct: Props<T> extends Base<T[]>
interface Base<T> {
  p?: T
}

interface Props<T> extends Base<T[]> {}

export function Row(props: Props<string>) {
  return null
}
