export interface ButtonProps {
  /** The button's visible label. */
  label: string
  /** Called when the button is clicked. */
  onClick?: () => void
  /** Visual style variant. */
  variant?: 'primary' | 'secondary'
}

export function Button({ label, onClick, variant = 'primary' }: ButtonProps) {
  return (
    <button onClick={onClick} data-variant={variant}>
      {label}
    </button>
  )
}
