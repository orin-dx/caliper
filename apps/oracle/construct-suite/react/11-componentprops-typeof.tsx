// construct: ComponentProps<typeof X> with a local component type
import type * as React from "react";

declare function Inner(props: { tone: "a" | "b" }): null;

interface Props {
  p?: React.ComponentProps<typeof Inner>;
}

export function Row(props: Props) {
  return null;
}
