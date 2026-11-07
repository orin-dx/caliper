// construct: A<string> & B<number>
type A<T> = { a?: T }
type B<T> = { b?: T }

interface Props {
  p?: A<string> & B<number>
}

export function Row(props: Props) {
  return null
}
