// construct: derived interface narrows an inherited property
interface Base {
  kind: string
  size?: number
}
interface Derived extends Base {
  kind: "derived"
  size: number
}
interface Props {
  p?: Derived
}
export function Row(props: Props) {
  return null
}
