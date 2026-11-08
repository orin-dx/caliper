// construct: two optional elements spread before a required one
interface Props {
  p?: [...[string?, number?], boolean]
}
export function Row(props: Props) {
  return null
}
