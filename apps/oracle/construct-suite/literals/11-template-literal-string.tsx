// construct: template literal type with string placeholder
import type * as React from "react";

export interface Props {
  p?: `data-${string}`;
}

export function Row(props: Props) {
  return null;
}
