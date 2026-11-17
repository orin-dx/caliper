// construct: type Both<T, U> = A<T> & B<U>
type A<T> = { a?: T }
type B<U> = { b?: U }
type Both<T, U> = A<T> & B<U>

interface Props {
  p?: Both<boolean, string[]>
}

export function Row(props: Props) {
  return null
}
