// construct: named properties alongside a string index signature
interface Props {
  p?: { id: string; [key: string]: string }
}
export function Row(props: Props) {
  return null
}
