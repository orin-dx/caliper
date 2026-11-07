// construct: LiteralUnion generic alias
import type * as React from "react";

type LiteralUnion<T extends U, U = string> = T | (U & {});

export interface Props {
  p?: LiteralUnion<"sm" | "lg">;
}

export function Row(props: Props) {
  return null;
}
