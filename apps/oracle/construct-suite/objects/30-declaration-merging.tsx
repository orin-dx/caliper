// construct: interface declaration merging
interface Merged {
  a: string
}
interface Merged {
  b: number
}
interface Props {
  p?: Merged
}
export function Row(props: Props) {
  return null
}
