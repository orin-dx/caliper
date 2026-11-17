// construct: a local declaration with the name another file exports, the other file imported by namespace
import type * as shared from './shared';

type Size = 'local';

interface Props {
  p: Size | shared.Size;
}

export function Row(props: Props) {
  return null;
}
