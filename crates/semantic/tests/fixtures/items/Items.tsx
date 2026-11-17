export interface AProps {
  a: string;
}

export interface BProps {
  b: number;
}

export interface AItemProps {
  itemOfA: boolean;
}

export interface BItemProps {
  itemOfB: number;
}

export function A(props: AProps) {
  return null;
}

export function B(props: BProps) {
  return null;
}

A.Item = (props: AItemProps) => null;

B.Item = (props: BItemProps) => null;
