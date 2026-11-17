// construct: conditional type depending on a type parameter
interface Props<T> {
  p?: T extends string ? "text" : "other"
}

export function Row(props: Props<string>) {
  return null
}
