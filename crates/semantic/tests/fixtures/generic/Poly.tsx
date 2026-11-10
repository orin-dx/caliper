import type * as React from "react";

type PolyProps<E extends React.ElementType> = {
  /** The element to render. */
  as?: E;
  tone?: "quiet" | "loud";
} & Omit<React.ComponentPropsWithoutRef<E>, "as" | "tone">;

/** Renders as a `div` unless `as` names another element. */
export function Poly<E extends React.ElementType = "div">(props: PolyProps<E>) {
  return null;
}

/** The same props as `Poly`, at another default. */
export function Anchor<E extends React.ElementType = "a">(props: PolyProps<E>) {
  return null;
}

interface Settings {
  dense: boolean;
}

/** Its default is an interface the module does not export, which the checker prints by a name no other file sees. */
export function Configured<T extends object = Settings>(props: { value: T; onChange?: (next: T) => void }) {
  return null;
}

/** No default: no instantiation is the one `<Listing />` renders, so the parameter stays open. */
export function Listing<T extends { id: string }>(props: { items: T[]; selected?: T["id"] }) {
  return null;
}
