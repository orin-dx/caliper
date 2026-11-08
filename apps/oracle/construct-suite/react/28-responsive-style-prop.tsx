// construct: responsive value: T | T[] | breakpoint record
type Breakpoint = "base" | "sm" | "md" | "lg";
type Responsive<T> = T | ReadonlyArray<T> | Partial<Record<Breakpoint, T>>;

interface Props {
  p?: Responsive<string | number>;
}

export function Row(props: Props) {
  return null;
}
