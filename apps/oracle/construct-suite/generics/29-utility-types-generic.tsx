// construct: Pick/Partial/Omit applied to a type parameter
interface Props<T extends { id: string; name: string }> {
  p?: Partial<Pick<T, "name">> & Omit<T, "name">
}

export function Row<T extends { id: string; name: string }>(props: Props<T>) {
  return null
}
