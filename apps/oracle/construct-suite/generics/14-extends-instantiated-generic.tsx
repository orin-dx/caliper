// construct: Props extends Base<number>
interface Base<T> {
  p?: T
}

interface Props extends Base<number> {}

export function Row(props: Props) {
  return null
}
