// construct: generic interface with default type parameter
interface Box<T = string> {
  value: T
  onChange?: (next: T) => void
}
interface Props {
  p?: Box<number>
}
export function Row(props: Props) {
  return null
}
