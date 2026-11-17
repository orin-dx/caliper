// construct: NamedExoticComponent / ForwardRefExoticComponent
import type * as React from "react";

interface Props {
  p?: React.NamedExoticComponent<{ n: number }> | React.ForwardRefExoticComponent<{ m: string }>;
}

export function Row(props: Props) {
  return null;
}
