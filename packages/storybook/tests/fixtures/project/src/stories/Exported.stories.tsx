import type { Meta } from '@storybook/react-vite'
import { List, ListItem } from '../components/List'

// A meta exported by name as the default: there is no `export default` declaration to rewrite.
const meta = { component: List, subcomponents: { ListItem } } satisfies Meta<typeof List>
export { meta as default }
