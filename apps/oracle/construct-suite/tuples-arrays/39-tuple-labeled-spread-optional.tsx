// construct: labeled optional element spread before a required one
interface Props {
  p?: [...[a?: string], b: number]
}
export function Row(props: Props) {
  return null
}
