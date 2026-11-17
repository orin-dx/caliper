// construct: type parameter named like a module-level interface
interface Item {
  a: string
}

type Wrap<Item> = Item[]

interface Props {
  p?: Wrap<number> | Item
}

export function Row(props: Props) {
  return null
}
