import type { Meta, StoryObj } from '@storybook/react-vite'
import { Badge } from '../components/Badge'

const meta = { component: Badge } satisfies Meta<typeof Badge>
export default meta

export const Basic: StoryObj<typeof meta> = { args: { label: 'New' } }
