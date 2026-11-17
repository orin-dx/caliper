// construct: numeric enum
import type * as React from "react";

enum Level {
  Low,
  Mid,
  High = 10,
}

export interface Props {
  p?: Level;
}

export function Row(props: Props) {
  return null;
}
