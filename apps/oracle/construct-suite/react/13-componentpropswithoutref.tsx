// construct: Omit<ComponentPropsWithoutRef<"a">, "href">
import type * as React from "react";

interface Props {
  p?: Omit<React.ComponentPropsWithoutRef<"a">, "href">;
}

export function Row(props: Props) {
  return null;
}
