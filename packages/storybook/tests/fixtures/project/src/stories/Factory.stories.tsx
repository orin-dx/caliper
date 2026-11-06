import preview from '#.storybook/preview'
import { List, ListItem } from '../components/List'

// A CSF factory: the meta is the argument of `preview.meta()`.
const meta = preview.meta({ component: List, subcomponents: { ListItem } })
export const Basic = meta.story({})
