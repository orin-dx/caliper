interface Base { /** The id. */ id: string }
interface Tagged { /** @default 'md' */ size?: string; /** @deprecated use scale */ legacy?: number }
interface Pair { /** a */ a: string; /** b */ b: number }
interface Many {
  /** One. */ one: string; /** Two. */ two: string
}
interface Child extends Base { /** Its tone. */ tone?: string }
interface Both { /** @default 1 @deprecated gone */ both?: number }

export function One(props: Base) {
  return null
}
export function Two(props: Tagged) {
  return null
}
export function Three(props: Pair) {
  return null
}
export function Four(props: Many) {
  return null
}
export function Five(props: { /** Inline. */ inline: string }) {
  return null
}
export function Six(props: Child) {
  return null
}
export function Seven(props: Both) {
  return null
}

