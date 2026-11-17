// construct: a default `defaultProps` gives the prop
interface Props {
  p?: 'sm' | 'md'
}
export function Row(props: Props) {
  return null
}
Row.defaultProps = { p: 'md' }
