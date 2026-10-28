import type { Meta } from '@storybook/react-vite'
import { Select } from '../components/Select'

// A member of a named import: Storybook's own resolver reads only a member of `import * as`.
const meta = { component: Select.Option } satisfies Meta<typeof Select.Option>
export default meta
