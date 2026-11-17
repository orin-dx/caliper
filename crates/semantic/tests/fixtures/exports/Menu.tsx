export interface MenuProps {
  open: boolean;
  label: string;
}

export interface ItemProps {
  value: number;
}

function Item(props: ItemProps) {
  return null;
}

function Menu(props: MenuProps) {
  return null;
}

Menu.Item = Item;

export default Menu;

export { default as Elsewhere } from "./Card";
