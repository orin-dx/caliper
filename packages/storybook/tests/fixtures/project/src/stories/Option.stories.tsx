import type { Meta } from '@storybook/react-vite'
import { Select } from '../components/Select'

// A member of a named import as the meta component.
const meta = { component: Select.Option } satisfies Meta<typeof Select.Option>
export default meta
