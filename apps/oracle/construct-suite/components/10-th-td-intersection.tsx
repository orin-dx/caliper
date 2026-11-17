// construct: props typed by a th and a td attribute intersection, as MUI's TableCell is; no single element to inherit from
// prop: className
import type { TdHTMLAttributes, ThHTMLAttributes } from 'react';

type Props = ThHTMLAttributes<HTMLTableCellElement> & TdHTMLAttributes<HTMLTableCellElement>;

export function Row(props: Props) {
  return null;
}
