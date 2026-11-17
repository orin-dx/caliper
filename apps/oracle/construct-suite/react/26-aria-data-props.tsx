// construct: aria-* and data-* hyphenated property keys
import type * as React from "react";

interface Props {
  p?: {
    "aria-label"?: string;
    "aria-hidden"?: boolean | "true" | "false";
    "aria-live"?: React.AriaAttributes["aria-live"];
    "data-testid"?: string;
    role?: React.AriaRole;
  };
}

export function Row(props: Props) {
  return null;
}
