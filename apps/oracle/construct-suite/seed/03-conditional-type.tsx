type Unwrap<T> = T extends Promise<infer U> ? U : T

interface Props {
  p: Unwrap<Promise<string>>
}

export function Row(props: Props) {
  return null
}
