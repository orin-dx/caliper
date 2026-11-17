// construct: ReactElement<P, T> with explicit type arguments
import type * as React from "react";

interface Props {
  p?: React.ReactElement<{ label: string }, "div">;
}

export function Row(props: Props) {
  return null;
}
