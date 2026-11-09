interface Props {
  /** One leading underscore is written as it is. */
  _single?: number;
  /** Two leading underscores: the checker's escaped name has a third. */
  __size?: string;
  /** Three leading underscores. */
  ___triple?: boolean;
  plain: string;
}

export function Markers(props: Props) {
  return null;
}

interface CallbackProps {
  /** An optional parameter, then a rest parameter. */
  format?: (percent?: number, ...rest: string[]) => string;
  close?: (...args: any[]) => void;
  onClick?: (e?: Event) => void;
  /** A required parameter, then an optional one. */
  plain: (a: number, b?: string) => void;
  /** `undefined` written in the type, which is not the same as optional. */
  explicit: (a: number, b: string | undefined) => void;
}

export function Callbacks(props: CallbackProps) {
  return null;
}

type Handler = (value?: string) => void;

type Overloaded = {
  (a?: string): string;
  (a: number, b?: number): number;
};

/** An interface is printed as its name whatever flags the checker is given, so its parameters cannot be read. */
interface NamedCallable {
  (a?: string): void;
}

interface AliasedProps {
  /** An optional parameter behind an alias: the alias is printed as its name, not as a parameter list. */
  onChange: Handler;
  /** A long parameter type, which the checker cuts short unless it is asked not to. */
  long: (a?: { a00xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a01xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a02xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a03xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a04xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a05xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a06xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a07xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a08xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a09xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a10xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; a11xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string }, b?: { b00xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b01xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b02xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b03xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b04xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b05xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b06xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b07xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b08xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b09xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b10xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string; b11xxxxxxxxxxxxxxxxxxxxxxxxxxxx: string }, c?: number, d?: string) => void;
  /** A `this` parameter is printed but is not one of the signature's parameters. */
  withThis: (this: Window, a?: number) => void;
  /** An overload set, and a function with a property of its own: each signature has its own markers. */
  overloaded: Overloaded;
  withCall: { (a?: string): number; displayName: string };
  ctor: { new (a?: string): Date };
  /** A callable interface, whose optional parameter cannot be told from a required one. */
  named: NamedCallable;
}

export function Aliased(props: AliasedProps) {
  return null;
}
