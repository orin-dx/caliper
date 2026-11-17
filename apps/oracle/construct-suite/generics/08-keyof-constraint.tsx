// construct: K extends keyof T
interface Props<T, K extends keyof T> {
  p?: K
}

export function Row<T, K extends keyof T>(props: Props<T, K>) {
  return null
}
