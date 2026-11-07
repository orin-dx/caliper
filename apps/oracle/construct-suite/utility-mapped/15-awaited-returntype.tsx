// construct: Awaited<ReturnType<F>>
type Load = () => Promise<{ items: string[] }>
interface Props { p?: Awaited<ReturnType<Load>> }
export function Row(props: Props) { return null }
