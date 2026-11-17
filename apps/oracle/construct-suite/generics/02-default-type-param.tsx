// construct: default type parameter, Props used bare
interface Props<T = string> {
  p?: T
}

export function Row(props: Props) {
  return null
}
