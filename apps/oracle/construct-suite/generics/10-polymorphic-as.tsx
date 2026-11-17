// construct: polymorphic as?: C & ComponentPropsWithoutRef<C>
import type * as React from "react"

type PolyProps<C extends React.ElementType> = {
  as?: C
} & React.ComponentPropsWithoutRef<C>

interface Props {
  p?: PolyProps<"button">
}

export function Row(props: Props) {
  return null
}
