// construct: union of enum members
import type * as React from "react";

enum Size {
  Small = "small",
  Medium = "medium",
  Large = "large",
}

export interface Props {
  p?: Size.Small | Size.Large;
}

export function Row(props: Props) {
  return null;
}
