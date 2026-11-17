// construct: two interfaces referencing each other
interface Folder {
  name: string
  files?: File[]
}

interface File {
  name: string
  parent?: Folder
}

interface Props {
  p?: Folder
}

export function Row(props: Props) {
  return null
}
