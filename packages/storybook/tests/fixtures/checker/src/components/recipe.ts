import type { FC } from 'react'

/** Makes a component from its default props, as a styling library's factory does: its props are inferred from them. */
function recipe<P>(defaults: P): FC<Partial<P>> {
  return () => {
    void defaults
    return null
  }
}

export const StyledBadge = recipe({ label: 'New', tone: 'info' as 'info' | 'warning' })
