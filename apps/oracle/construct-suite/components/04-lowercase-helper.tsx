// construct: a component passed through a lowercase helper that returns it unchanged
interface Props {
  p: 'helped';
}

function withNothing<P>(component: (props: P) => null): (props: P) => null {
  return component;
}

export const Row = withNothing((props: Props) => {
  return null;
});
