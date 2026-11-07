// construct: Parameters<F>[0]
type Handler = (event: { x: number }, extra: string) => void
interface Props { p?: Parameters<Handler>[0] }
export function Row(props: Props) { return null }
