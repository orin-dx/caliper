// construct: keyof typeof value
declare const sizes: { sm: 1; md: 2; lg: 3 }
interface Props { p?: keyof typeof sizes }
export function Row(props: Props) { return null }
