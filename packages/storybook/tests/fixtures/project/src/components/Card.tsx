import type { ReactNode } from 'react'

type Tone = 'neutral' | 'info' | 'danger'

interface CardProps {
  title: string
  tone?: Tone
  tags?: string[]
  meta?: { author: string; date?: string }
  elevation?: 0 | 1 | 2
  footer?: ReactNode
  onDismiss?: () => void
}

/**
 * A surface that groups content.
 * @summary Grouped content
 */
export default function Card({
  title,
  tone = 'neutral',
  tags = [],
  meta = { author: 'anon' },
  elevation = 1,
  footer,
  onDismiss,
}: CardProps) {
  return (
    <section data-tone={tone} data-elevation={elevation}>
      <h2>{title}</h2>
      <p>{meta.author}</p>
      <ul>
        {tags.map((tag) => (
          <li key={tag}>{tag}</li>
        ))}
      </ul>
      {footer}
      <button type="button" onClick={onDismiss}>
        x
      </button>
    </section>
  )
}
