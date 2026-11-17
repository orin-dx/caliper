// construct: generic Props with ElementType default, used bare
import type * as React from "react"

interface Props<C extends React.ElementType = "div"> {
  p?: { as?: C } & React.ComponentPropsWithoutRef<C>
}

export function Row(props: Props) {
  return null
}
