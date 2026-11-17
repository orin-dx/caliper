// construct: union with never (absorbed)
import type * as React from "react";

export interface Props {
  p?: "a" | "b" | never;
}

export function Row(props: Props) {
  return null;
}
