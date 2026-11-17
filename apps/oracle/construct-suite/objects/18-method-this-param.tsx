// construct: method with explicit this parameter
interface Props {
  p?: { run(this: { count: number }, step: number): void }
}
export function Row(props: Props) {
  return null
}
