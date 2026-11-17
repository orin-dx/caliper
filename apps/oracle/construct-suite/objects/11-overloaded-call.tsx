// construct: overloaded call signatures
interface Props {
  p?: {
    (value: string): string
    (value: number): number
  }
}
export function Row(props: Props) {
  return null
}
