// construct: polymorphic this type in members
interface Chain {
  next(): this
  value: number
}
interface Props {
  p?: Chain
}
export function Row(props: Props) {
  return null
}
