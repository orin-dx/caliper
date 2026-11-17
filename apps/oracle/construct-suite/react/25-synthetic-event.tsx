// construct: SyntheticEvent callback
import type * as React from "react";

interface Props {
  p?: (event: React.SyntheticEvent<HTMLFormElement>, reason: "submit" | "reset") => void;
}

export function Row(props: Props) {
  return null;
}
