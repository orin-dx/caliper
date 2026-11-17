// construct: export { RowImpl as Row }
// components: RowImpl=Row
interface Props {
  p: 'renamed';
}

function RowImpl(props: Props) {
  return null;
}

export { RowImpl as Row };
