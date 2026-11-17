// construct: distributive conditional type
type ToArray<T> = T extends unknown ? T[] : never
interface Props { p?: ToArray<string | number> }
export function Row(props: Props) { return null }
