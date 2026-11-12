import type { HTMLAttributes, RefAttributes } from 'react'
import type { BaseProps } from './Base'

export interface ButtonProps
  extends BaseProps,
    Omit<HTMLAttributes<HTMLButtonElement>, 'title'>,
    RefAttributes<HTMLButtonElement> {
  /**
   * Its tone.
   * @since 2.0
   * @see https://example.com
   */
  tone: 'zeta' | 'alpha'
}

export function Button(props: ButtonProps) {
  return null
}

export interface PickedProps extends Pick<BaseProps, 'disabled'> {
  inline?: number
}

export function Picked(props: PickedProps) {
  return null
}

type Extra = { label: string }

export function Card(props: Extra & { count?: number }) {
  return null
}
