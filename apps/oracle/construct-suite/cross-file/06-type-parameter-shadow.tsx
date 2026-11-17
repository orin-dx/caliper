// construct: a type parameter named like an imported alias, which it shadows inside the generic
import type { Size } from './shared';

type Wrap<Size> = { value: Size };

interface Props {
  p: Wrap<'param'>['value'] | Size;
}

export function Row(props: Props) {
  return null;
}
