// construct: generic interface instantiated through a type alias
interface Box<T> {
  value: T
  label?: string
}

type StringBox = Box<string>

interface Props {
  p?: StringBox
}

export function Row(props: Props) {
  return null
}
