import type { Meta, StoryObj } from '@storybook/react-vite'
import { List, ListItem } from '../components/List'

/** Lists of things. */
const meta = { component: List, subcomponents: { ListItem } } satisfies Meta<typeof List>
export default meta

export const Basic: StoryObj<typeof meta> = { args: { ordered: true } }
