// construct: a function prop whose parameters are lib.dom types
interface Props {
  p: (signal: AbortSignal, init?: RequestInit) => Promise<Response>;
}

export function Row(props: Props) {
  return null;
}
