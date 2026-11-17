// construct: T[K] indexed access
interface Props<T, K extends keyof T> {
  p?: T[K]
}

export function Row(props: Props<{ size: "sm" | "lg" }, "size">) {
  return null
}
