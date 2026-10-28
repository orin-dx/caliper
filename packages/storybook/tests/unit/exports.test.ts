import { describe, expect, it } from 'vitest'
import { localNameOf } from '../../src/exports.js'

describe('localNameOf', () => {
  it.each([
    ['export default Card;', 'Card'],
    ['export default Card', 'Card'],
    ['export default function Card(props) {}', 'Card'],
    ['export default async function Loader() {}', 'Loader'],
    ['export default class Card extends Component {}', 'Card'],
    ['export default React.forwardRef(InternalTable) as RefInternalTable', 'InternalTable'],
    ['export default memo(forwardRef<HTMLDivElement, Props>(Card))', 'Card'],
    ['export { Inner as default }', 'Inner'],
    ['export { Other, Inner as default, More }', 'Inner'],
  ])('reads the default export of %s', (source, expected) => {
    expect(localNameOf(source, 'default')).toBe(expected)
  })

  it('reads the local name a named export renames', () => {
    expect(localNameOf('export { Inner as Tag, Other }', 'Tag')).toBe('Inner')
    expect(localNameOf('export { Inner as Tag, Other }', 'Other')).toBeUndefined()
  })

  it('says nothing when the source does not name a local', () => {
    expect(localNameOf('export default makePanel()', 'default')).toBeUndefined()
    expect(localNameOf('export default { a: 1 }', 'default')).toBeUndefined()
    expect(localNameOf('export const Card = () => null', 'default')).toBeUndefined()
  })
})
