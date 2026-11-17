// construct: sx style-system prop as recursive plain object type
import type * as React from "react";

type SxValue = string | number | SxObject | undefined;
interface SxObject {
  [selector: string]: SxValue;
}
type Sx = SxObject | React.CSSProperties;

interface Props {
  p?: Sx | ReadonlyArray<Sx>;
}

export function Row(props: Props) {
  return null;
}
