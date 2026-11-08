// construct: generic self-referential node
interface TreeNode<T> {
  value: T
  children?: TreeNode<T>[]
}

interface Props {
  p?: TreeNode<string>
}

export function Row(props: Props) {
  return null
}
