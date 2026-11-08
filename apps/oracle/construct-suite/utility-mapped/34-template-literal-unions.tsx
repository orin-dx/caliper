// construct: template literal type from unions
type Side = 'top' | 'bottom'
type Align = 'left' | 'right'
interface Props { p?: `${Side}-${Align}` }
export function Row(props: Props) { return null }
