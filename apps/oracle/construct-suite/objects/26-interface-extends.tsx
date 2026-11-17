// construct: interface extending another interface
interface Base {
  id: string
}
interface Item extends Base {
  label: string
}
interface Props {
  p?: Item
}
export function Row(props: Props) {
  return null
}
