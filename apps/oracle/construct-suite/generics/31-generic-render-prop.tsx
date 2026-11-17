// construct: render prop with generic item and ReactNode result
import type * as React from "react"

interface Props<T> {
  p?: (item: T) => React.ReactNode
}

export function Row<T>(props: Props<T>) {
  return null
}
