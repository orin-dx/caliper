// construct: interface extending a utility-type instantiation
interface Base {
  a: string
  b: number
}
interface Rest extends Omit<Base, "a"> {
  c: boolean
}
interface Props {
  p?: Rest
}
export function Row(props: Props) {
  return null
}
