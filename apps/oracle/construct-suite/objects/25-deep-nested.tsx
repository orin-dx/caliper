// construct: deeply nested optional objects
interface Props {
  p?: { a?: { b?: { c?: { d: string } } } }
}
export function Row(props: Props) {
  return null
}
