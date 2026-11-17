// construct: self-referential object type
interface Tree {
  label: string
  children?: Tree[]
}
interface Props {
  p?: Tree
}
export function Row(props: Props) {
  return null
}
