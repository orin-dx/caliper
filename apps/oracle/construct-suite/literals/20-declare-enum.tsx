// construct: declare enum (ambient)
import type * as React from "react";

declare enum Kind {
  A = "a",
  B = "b",
}

export interface Props {
  p?: Kind;
}

export function Row(props: Props) {
  return null;
}
