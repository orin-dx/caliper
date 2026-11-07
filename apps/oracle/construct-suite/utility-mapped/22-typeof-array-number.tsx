// construct: (typeof const array)[number]
declare const sizes: readonly ['sm', 'md', 'lg']
interface Props { p?: (typeof sizes)[number] }
export function Row(props: Props) { return null }
