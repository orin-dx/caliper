// construct: LiteralUnion pattern (string & {})
import type * as React from "react";

export interface Props {
  p?: "red" | "green" | (string & {});
}

export function Row(props: Props) {
  return null;
}
