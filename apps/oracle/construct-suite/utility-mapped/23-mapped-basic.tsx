// construct: mapped type over union
type Keys = 'a' | 'b' | 'c'
type Flags = { [K in Keys]: boolean }
interface Props { p?: Flags }
export function Row(props: Props) { return null }
