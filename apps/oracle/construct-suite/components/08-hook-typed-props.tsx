// construct: props typed by the return type of a lowercase hook
function useRowState() {
  return { p: 'hooked' as 'hooked' | 'idle' };
}

type Props = ReturnType<typeof useRowState>;

export function Row(props: Props) {
  return null;
}
