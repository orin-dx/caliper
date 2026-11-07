// construct: unique-symbol keyed member
declare const tag: unique symbol
interface Props {
  p?: { [tag]: string; name: string }
}
export function Row(props: Props) {
  return null
}
