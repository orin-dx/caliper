// construct: a prop an extended interface declares, so `parent` is that interface
interface Base {
  /** From the base. */
  p: string
}
interface Props extends Base {
  q?: number
}
export function Row(props: Props) {
  return null
}
