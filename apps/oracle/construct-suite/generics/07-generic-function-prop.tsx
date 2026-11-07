// construct: prop whose type is itself a generic function
interface Props {
  p?: <U>(item: U) => U
}

export function Row(props: Props) {
  return null
}
