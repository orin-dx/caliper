// construct: ReturnType<F>
type Make = (n: number) => { id: string; n: number }
interface Props { p?: ReturnType<Make> }
export function Row(props: Props) { return null }
