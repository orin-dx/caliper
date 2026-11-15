// construct: props of a context's Provider, which only @types/react declares (`value`, `children`)
// prop: value
import type { ProviderProps } from 'react';

export function Row(props: ProviderProps<'compact' | 'roomy'>) {
  return null;
}
