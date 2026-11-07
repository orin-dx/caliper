// construct: recursive JSON value alias
type Json = string | number | boolean | null | Json[] | { [key: string]: Json }

interface Props {
  p?: Json
}

export function Row(props: Props) {
  return null
}
