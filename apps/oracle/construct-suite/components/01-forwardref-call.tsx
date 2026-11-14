// construct: a component made by a forwardRef call, typed by its arguments
import * as React from 'react';

interface Props {
  p: 'forwarded';
}

export const Row = React.forwardRef<HTMLDivElement, Props>((props, ref) => {
  return null;
});
