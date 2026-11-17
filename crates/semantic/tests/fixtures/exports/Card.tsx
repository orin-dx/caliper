export interface CardProps {
  title: string;
}

function CardImpl(props: CardProps) {
  return null;
}

export { CardImpl as Card };

export interface OtherProps {
  other: number;
}

export default function Other(props: OtherProps) {
  return null;
}
