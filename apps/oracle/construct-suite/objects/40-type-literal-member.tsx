// construct: a prop of a type literal, which no interface declares, so it has no `parent`
type Props = {
  p: string
}
export function Row(props: Props) {
  return null
}
