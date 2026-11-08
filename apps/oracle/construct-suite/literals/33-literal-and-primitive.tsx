// construct: string intersected with literal union
import type * as React from "react";

export interface Props {
  p?: string & ("x" | "y");
}

export function Row(props: Props) {
  return null;
}
