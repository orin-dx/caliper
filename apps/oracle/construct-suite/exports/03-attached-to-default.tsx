// construct: Menu.Item = Item beside export default Menu
// components: Item=default.Item Menu=default
interface ItemProps {
  p: 'item';
}

interface MenuProps {
  p: 'menu';
}

function Item(props: ItemProps) {
  return null;
}

function Menu(props: MenuProps) {
  return null;
}

Menu.Item = Item;

export default Menu;
