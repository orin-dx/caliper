import type { ReactNode } from 'react'

export interface ListProps {
  /** Ordered or not. */
  ordered?: boolean
  children?: ReactNode
}

export function List({ ordered = false, children }: ListProps) {
  return ordered ? <ol>{children}</ol> : <ul>{children}</ul>
}

export interface ListItemProps {
  /** Shown before the text. */
  bullet?: string
  selected?: boolean
  children?: ReactNode
}

export function ListItem({ bullet = '•', selected, children }: ListItemProps) {
  return (
    <li aria-selected={selected}>
      {bullet} {children}
    </li>
  )
}
