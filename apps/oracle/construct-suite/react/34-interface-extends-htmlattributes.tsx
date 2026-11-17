// construct: helper interface extending HTMLAttributes with Omit
import type * as React from "react";

interface BoxProps extends Omit<React.HTMLAttributes<HTMLDivElement>, "color"> {
  color?: "red" | "blue";
  as?: React.ElementType;
}

interface Props {
  p?: BoxProps;
}

export function Row(props: Props) {
  return null;
}
