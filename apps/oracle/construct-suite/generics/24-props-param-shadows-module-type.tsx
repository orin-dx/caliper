// construct: Props<Item> where Item shadows a module interface
interface Item {
  a: string
}

interface Props<Item> {
  p?: Item
}

export function Row(props: Props<number>) {
  const unused: Item = { a: "x" }
  void unused
  return null
}
