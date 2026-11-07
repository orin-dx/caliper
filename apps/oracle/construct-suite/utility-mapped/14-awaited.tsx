// construct: Awaited<T>
interface Props { p?: Awaited<Promise<Promise<string>>> }
export function Row(props: Props) { return null }
