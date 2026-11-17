// construct: self-referential tree node
interface TreeNode {
  label: string
  children?: TreeNode[]
}

interface Props {
  p?: TreeNode
}

export function Row(props: Props) {
  return null
}
