// construct: string enum
import type * as React from "react";

enum Intent {
  Info = "info",
  Danger = "danger",
}

export interface Props {
  p?: Intent;
}

export function Row(props: Props) {
  return null;
}
