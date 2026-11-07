// construct: React.ComponentType<P>
import type * as React from "react";

interface Props {
  p?: React.ComponentType<{ id: string }>;
}

export function Row(props: Props) {
  return null;
}
