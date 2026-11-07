// construct: nested object literal types
interface Props {
  p?: {
    size: { width: number; height: number }
    pos?: { x: number; y: number }
  }
}
export function Row(props: Props) {
  return null
}
