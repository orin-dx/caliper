// construct: Parameters<F>
type Handler = (id: string, flag?: boolean) => void
interface Props { p?: Parameters<Handler> }
export function Row(props: Props) { return null }
