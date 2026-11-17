// construct: Exclude<U, X>
type Size = 'xs' | 'sm' | 'md' | 'lg'
interface Props { p?: Exclude<Size, 'xs'> }
export function Row(props: Props) { return null }
