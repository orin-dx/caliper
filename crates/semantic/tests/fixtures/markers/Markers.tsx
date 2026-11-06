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
