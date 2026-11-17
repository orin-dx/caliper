// construct: export default React.forwardRef(Inner) as T
// components: Inner=default
import * as React from 'react';

interface Props {
  p: 'wrapped';
}

const Inner = (props: Props) => {
  return null;
};

export default React.forwardRef(Inner) as typeof Inner;
