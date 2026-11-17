// construct: props that inherit `p` from an interface in another file
import type { BaseProps } from './shared';

interface Props extends BaseProps {}

export function Row(props: Props) {
  return null;
}
