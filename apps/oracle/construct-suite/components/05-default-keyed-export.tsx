// construct: a component exported under the `default` key of an export list, not by `export default`
// components: Row=default
interface Props {
  p: 'keyed';
}

function Row(props: Props) {
  return null;
}

export { Row as default };
