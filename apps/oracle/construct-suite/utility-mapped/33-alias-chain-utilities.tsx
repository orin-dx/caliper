// construct: alias chain Readonly<Pick<Partial<B>, K>>
interface B { a: string; b: number; c: boolean }
type Step1 = Partial<B>
type Step2 = Pick<Step1, 'a' | 'b'>
type Final = Readonly<Step2>
interface Props { p: Final }
export function Row(props: Props) { return null }
