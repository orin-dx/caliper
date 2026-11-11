// construct: a default export beside a named one
// components: Row=Row Other=default
interface Props {
  p: 'named';
}

interface OtherProps {
  p: 'default';
}

export function Row(props: Props) {
  return null;
}

export default function Other(props: OtherProps) {
  return null;
}
