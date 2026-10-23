import type { Meta, StoryObj } from '@storybook/react-vite'
import Card from '../components/Card'

const meta = { component: Card } satisfies Meta<typeof Card>
export default meta

export const Basic: StoryObj<typeof meta> = { args: { title: 'Hello' } }
