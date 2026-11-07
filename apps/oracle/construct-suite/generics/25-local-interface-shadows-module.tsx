// construct: function-local interface with same name as module-level one
interface Item {
  a: string
}

interface Props {
  p?: Item
}

export function Row(props: Props) {
  interface Item {
    b: number
  }
  const local: Item = { b: 1 }
  void local
  return null
}
