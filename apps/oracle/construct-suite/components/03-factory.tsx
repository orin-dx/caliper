// construct: a component returned by a generic factory, its props given as the type argument
interface Props {
  p: 'made';
}

function createComponent<P>() {
  return (props: P) => null;
}

export const Row = createComponent<Props>();
