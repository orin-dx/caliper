// construct: intersection of literal unions
import type * as React from "react";

type A = "a" | "b" | "c";
type B = "b" | "c" | "d";

export interface Props {
  p?: A & B;
}

export function Row(props: Props) {
  return null;
}
