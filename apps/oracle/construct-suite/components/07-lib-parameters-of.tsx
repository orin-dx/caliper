// construct: a prop typed by a lib function's parameter, through Parameters<typeof …>
interface Props {
  p: Parameters<typeof setTimeout>[1];
}

export function Row(props: Props) {
  return null;
}
