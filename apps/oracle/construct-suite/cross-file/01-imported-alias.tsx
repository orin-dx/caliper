// construct: a union alias imported from another file
import type { Size } from './shared';

interface Props {
  p: Size;
}

export function Row(props: Props) {
  return null;
}
