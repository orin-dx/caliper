// construct: PropsWithChildren<P>
import type * as React from "react";

interface Props {
  p?: React.PropsWithChildren<{ tone: "info" | "warn" }>;
}

export function Row(props: Props) {
  return null;
}
