// construct: merged enum declarations
import type * as React from "react";

enum Color {
  Red = "red",
}
enum Color {
  Blue = "blue",
}

export interface Props {
  p?: Color;
}

export function Row(props: Props) {
  return null;
}
