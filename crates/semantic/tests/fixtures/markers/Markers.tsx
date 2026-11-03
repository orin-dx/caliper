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
