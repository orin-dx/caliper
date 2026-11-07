// construct: quoted and numeric-literal property names
interface Props {
  p?: { 1: string; "aria-label": string; 'x y'?: number }
}
export function Row(props: Props) {
  return null
}
