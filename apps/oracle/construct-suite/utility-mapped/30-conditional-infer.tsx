// construct: conditional type with infer
type Elem<T> = T extends readonly (infer U)[] ? U : T
interface Props { p?: Elem<string[]> }
export function Row(props: Props) { return null }
