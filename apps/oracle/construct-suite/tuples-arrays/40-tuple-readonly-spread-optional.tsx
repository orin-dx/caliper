// construct: readonly tuple with an optional element spread before a required one
interface Props {
  p?: readonly [...[string?], number]
}
export function Row(props: Props) {
  return null
}
