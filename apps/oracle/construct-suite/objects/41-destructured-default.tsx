// construct: a default the component's destructured parameter gives the prop
interface Props {
  p?: 'sm' | 'md'
}
export function Row({ p = 'md' }: Props) {
  return null
}
