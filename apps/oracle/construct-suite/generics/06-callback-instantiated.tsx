// construct: generic callback instantiated by concrete arg
interface Props<T> {
  p?: (value: T, index: number) => boolean
}

export function Row(props: Props<string>) {
  return null
}
