// Declarations the cross-file rows import; not a row (a fixture is a `.tsx`).
export type Size = 'sm' | 'md' | 'lg';

export interface BaseProps {
  /** The tone, declared in another file. */
  p: 'base';
}

export interface Options {
  dense: boolean;
  size: Size;
}
