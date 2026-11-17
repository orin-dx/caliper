// construct: const enum
import type * as React from "react";

const enum Mode {
  Read = "read",
  Write = "write",
}

export interface Props {
  p?: Mode;
}

export function Row(props: Props) {
  return null;
}
