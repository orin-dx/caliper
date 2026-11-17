// construct: get/set accessors in an interface
interface Box {
  get value(): string
  set value(next: string)
}
interface Props {
  p?: Box
}
export function Row(props: Props) {
  return null
}
