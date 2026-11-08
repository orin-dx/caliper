// construct: enum member type (E.A)
import type * as React from "react";

enum Size {
  Small = "small",
  Large = "large",
}

export interface Props {
  p?: Size.Small;
}

export function Row(props: Props) {
  return null;
}
