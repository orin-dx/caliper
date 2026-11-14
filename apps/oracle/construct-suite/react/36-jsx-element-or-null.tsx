// construct: JSX.Element reached through the React namespace, beside null
import type * as React from "react";

interface Props {
  p: React.JSX.Element | null;
}

export function Row(props: Props) {
  return null;
}
