// construct: union of ReactNode and render function
import type * as React from "react";

interface Props {
  p?: React.ReactNode | ((open: boolean) => React.ReactNode);
}

export function Row(props: Props) {
  return null;
}
