// construct: alias of an alias, each fixing one parameter
type Pair<A, B> = { first: A; second: B }
type StringPair<B> = Pair<string, B>
type StringNumber = StringPair<number>

interface Props {
  p?: StringNumber
}

export function Row(props: Props) {
  return null
}
