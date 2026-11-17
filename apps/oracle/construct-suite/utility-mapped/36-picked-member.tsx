// construct: a prop `Pick` maps from an interface, which still declares it, so `parent` is that interface
interface Base {
  /**
   * Picked from the base.
   * @since 1.0
   */
  p?: number
  q: string
}
type Props = Pick<Base, 'p'>
export function Row(props: Props) {
  return null
}
