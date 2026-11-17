// construct: unique symbol
import type * as React from "react";

declare const brand: unique symbol;

export interface Props {
  p?: typeof brand;
}

export function Row(props: Props) {
  return null;
}
