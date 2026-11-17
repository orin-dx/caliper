// construct: polymorphic as with Omit<ComponentPropsWithoutRef<C>, keyof Own>
import type * as React from "react"

interface Own {
  size?: "sm" | "lg"
}

type PolyProps<C extends React.ElementType> = Own & {
  as?: C
} & Omit<React.ComponentPropsWithoutRef<C>, keyof Own | "as">

interface Props {
  p?: PolyProps<"a">
}

export function Row(props: Props) {
  return null
}
