import type { Meta, StoryObj } from '@storybook/react-vite'
import Menu from '../components/Menu'

// A default export that is `Object.assign(Root, { Item })`: the story's `Menu` is the component declared as `Root`.
const meta = { component: Menu } satisfies Meta<typeof Menu>
export default meta

export const Basic: StoryObj<typeof meta> = { args: { open: true } }
