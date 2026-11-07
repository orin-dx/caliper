// construct: generic variadic tuple alias instantiation
type Prepend<T, U extends unknown[]> = [T, ...U]
interface Props {
  p?: Prepend<string, [number, boolean]>
}
export function Row(props: Props) {
  return null
}
