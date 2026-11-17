// construct: variadic tuple spreading a tuple alias
type Tail = [number, boolean]
interface Props {
  p?: [string, ...Tail]
}
export function Row(props: Props) {
  return null
}
