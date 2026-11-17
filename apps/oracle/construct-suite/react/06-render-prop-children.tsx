// construct: children as render function
import type * as React from "react";

interface Props {
  p: (state: { open: boolean; toggle: () => void }) => React.ReactNode;
}

export function Row(props: Props) {
  return null;
}
