// construct: a union alias imported through a renaming re-export
import type { Tone } from './barrel';

interface Props {
  p: Tone;
}

export function Row(props: Props) {
  return null;
}
