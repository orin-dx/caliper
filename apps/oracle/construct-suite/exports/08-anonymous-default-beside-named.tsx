// construct: an anonymous default export beside a named export
// components: Named=Named Unnamed=default
interface NamedProps {
  p: 'named';
}

interface DefaultProps {
  p: 'default';
}

export function Named(props: NamedProps) {
  return null;
}

export default function (props: DefaultProps) {
  return null;
}
