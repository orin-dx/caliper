// construct: mixed literal union (string, number, boolean)
import type * as React from "react";

export interface Props {
  p?: "auto" | 0 | 100 | true;
}

export function Row(props: Props) {
  return null;
}
