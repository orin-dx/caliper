// construct: literal union with null and undefined
import type * as React from "react";

export interface Props {
  p: "on" | "off" | null | undefined;
}

export function Row(props: Props) {
  return null;
}
