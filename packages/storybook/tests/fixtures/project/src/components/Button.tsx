import type { MouseEvent, ReactNode } from 'react'

export interface ButtonProps {
  /** The text on the button. */
  label: string
  /** Visual weight. */
  variant?: 'primary' | 'secondary' | 'ghost'
  size?: 'sm' | 'md' | 'lg'
  disabled?: boolean
  /** How many times it may be pressed. */
  count?: number
  onClick?: (event: MouseEvent<HTMLButtonElement>) => void
  icon?: ReactNode
  /** @deprecated use `variant` */
  primary?: boolean
}

/** A button for actions. */
export function Button({
  label,
  variant = 'primary',
  size = 'md',
  disabled = false,
  count = 3,
  onClick,
  icon,
}: ButtonProps) {
  return (
    <button
      type="button"
      data-variant={variant}
      data-size={size}
      disabled={disabled}
      data-count={count}
      onClick={onClick}
    >
      {icon}
      {label}
    </button>
  )
}
