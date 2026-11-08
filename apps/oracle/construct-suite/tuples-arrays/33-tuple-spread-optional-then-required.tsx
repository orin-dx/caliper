// construct: optional element spread before a required one
interface Props {
  p?: [...[string?], number]
}
export function Row(props: Props) {
  return null
}
