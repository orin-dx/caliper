// construct: call and construct signatures on one object type
interface Props {
  p?: {
    (value: number): string
    new (value: number): { n: number }
  }
}
export function Row(props: Props) {
  return null
}
