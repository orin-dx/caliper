// construct: Required<T>
interface Base { a?: string; b?: number }
interface Props { p?: Required<Base> }
export function Row(props: Props) { return null }
