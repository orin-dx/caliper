import type { Missing } from "does-not-exist";

interface Props {
  /** Written `any`: the checker's `any`, with nothing wrong. */
  explicitAny: any;
  /** The import does not resolve, so the checker's error type stands in. */
  fromImport: Missing;
  /** The name is declared nowhere. */
  undeclared: NotDeclaredAnywhere;
  label: string;
}

export function Errors(props: Props) {
  return null;
}
