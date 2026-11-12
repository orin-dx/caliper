import type { ComponentProps } from 'react'
import { StyledBadge } from './recipe'

/** A badge whose props only the TypeScript checker can read: a factory call infers them. */
export function Badge(props: ComponentProps<typeof StyledBadge>) {
  return <StyledBadge {...props} />
}
