// construct: interface extending several interfaces
interface A {
  a: string
}
interface B {
  b: number
}
interface Both extends A, B {
  c?: boolean
}
interface Props {
  p?: Both
}
export function Row(props: Props) {
  return null
}
