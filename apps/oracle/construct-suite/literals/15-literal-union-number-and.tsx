// construct: LiteralUnion pattern (number & {})
import type * as React from "react";

export interface Props {
  p?: 1 | 2 | (number & {});
}

export function Row(props: Props) {
  return null;
}
