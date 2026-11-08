// construct: generic Props<T extends string>
export interface Props<T extends string> {
  p?: T
}

export function Row<T extends string>(props: Props<T>) {
  return null
}
