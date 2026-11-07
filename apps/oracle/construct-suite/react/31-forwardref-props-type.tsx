// construct: PropsWithoutRef<P> & RefAttributes<T>
import type * as React from "react";

interface InnerProps {
  label: string;
}

interface Props {
  p?: React.PropsWithoutRef<InnerProps> & React.RefAttributes<HTMLButtonElement>;
}

export function Row(props: Props) {
  return null;
}
