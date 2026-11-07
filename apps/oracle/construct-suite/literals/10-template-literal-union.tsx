// construct: template literal type over unions (expands)
import type * as React from "react";

type Side = "top" | "bottom";
type Align = "left" | "right";

export interface Props {
  p?: `${Side}-${Align}`;
}

export function Row(props: Props) {
  return null;
}
