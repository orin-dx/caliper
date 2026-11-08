// construct: method returning polymorphic this
interface Chain {
  next(): this
  id: string
}

interface Props {
  p?: Chain
}

export function Row(props: Props) {
  return null
}
