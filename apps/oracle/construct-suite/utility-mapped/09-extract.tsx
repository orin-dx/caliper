// construct: Extract<U, X>
type Val = string | number | boolean | null
interface Props { p?: Extract<Val, string | number> }
export function Row(props: Props) { return null }
