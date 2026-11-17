import type { Meta } from '@storybook/react-vite'
import { List, ListItem } from '../components/List'

// A subcomponent read through a local object: the resolver follows the binding declared in this file.
const parts = { ListItem }
const meta = { component: List, subcomponents: { ListItem: parts.ListItem } } satisfies Meta<typeof List>
export default meta
