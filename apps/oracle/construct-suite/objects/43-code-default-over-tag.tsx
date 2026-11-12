// construct: a code default and a different `@default` tag: the code's wins, as in react-docgen-typescript
interface Props {
  /** @default 'sm' */
  p?: 'sm' | 'md'
}
export function Row({ p = 'md' }: Props) {
  return null
}
