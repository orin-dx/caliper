interface RootProps {
  /** Whether the menu shows its items. */
  open?: boolean
}

interface ItemProps {
  /** The text of the item. */
  label: string
}

const Root = ({ open = false }: RootProps) => <ul hidden={!open} />

const Item = ({ label }: ItemProps) => <li>{label}</li>

export default Object.assign(Root, { Item })
