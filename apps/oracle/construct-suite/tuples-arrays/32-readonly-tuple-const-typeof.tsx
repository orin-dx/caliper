// construct: typeof const-asserted tuple
declare const sizes: readonly ['sm', 'md']
interface Props {
  p?: typeof sizes
}
export function Row(props: Props) {
  return null
}
