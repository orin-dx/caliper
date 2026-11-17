// construct: explicit this parameter in a callback
interface Props {
  p?: (this: { id: string }, value: number) => void
}

export function Row(props: Props) {
  return null
}
