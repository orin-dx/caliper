// construct: computed property names with literal keys
interface Props {
  p?: { ["a-b"]: string; ["c"]?: number }
}
export function Row(props: Props) {
  return null
}
