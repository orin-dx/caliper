import type { Meta } from '@storybook/react-vite'
import { Select } from '../components/Select'

// A function-valued member of a named import as a subcomponent: Storybook's resolver reads only members of `import * as`.
const meta = { component: Select, subcomponents: { Option: Select.Option } } satisfies Meta<typeof Select>
export default meta
