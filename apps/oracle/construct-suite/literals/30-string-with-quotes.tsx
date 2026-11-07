// construct: string literals containing quotes and backslash
import type * as React from "react";

export interface Props {
  p?: 'say "hi"' | "it's" | "back\\slash" | "";
}

export function Row(props: Props) {
  return null;
}
