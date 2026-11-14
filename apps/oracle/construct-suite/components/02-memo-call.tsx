// construct: a component made by a memo call over a named function
import * as React from 'react';

interface Props {
  p: 'memoized';
}

export const Row = React.memo(function Row(props: Props) {
  return null;
});
