import type { HTMLAttributes, RefAttributes } from 'react'

interface DivProps<T> extends HTMLAttributes<HTMLDivElement> {
  value: T
}

export function Direct(props: DivProps<string>) {
  return null
}

interface ButtonBase<T> extends HTMLAttributes<HTMLButtonElement> {
  item: T
}

interface Chain<T> extends ButtonBase<T>, RefAttributes<HTMLButtonElement> {
  value: T
}

export function Chained(props: Chain<number>) {
  return null
}

interface Plain<T> {
  value: T
}

interface Own<T> extends Plain<T> {
  label: string
}

export function Bare(props: Own<string>) {
  return null
}

export function Mixed(props: DivProps<boolean> & { extra?: number }) {
  return null
}

export interface Fixed extends HTMLAttributes<HTMLSpanElement> {
  size?: number
}

export function Plainly(props: Fixed) {
  return null
}
