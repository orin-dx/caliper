// construct: default type parameter referencing an earlier one
interface Props<T, U = T[]> {
  p?: U
}

export function Row(props: Props<number>) {
  return null
}
