type Size = "sm" | "lg";

interface Props {
  /** Visible text. */
  label: string;
  size?: Size;
  disabled?: boolean;
  onClick?: (event: number) => void;
  tags?: string[];
  pair: [string, number];
  box: { a: number; b?: string };
}

export function Button({ size = "sm", ...props }: Props) {
  return null;
}

export default function Other(props: { a?: number }) {
  return null;
}

function Hidden(props: { x: number }) {
  return null;
}
