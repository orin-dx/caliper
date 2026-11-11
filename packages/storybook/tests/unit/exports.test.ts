import { describe, expect, it } from 'vitest'
import { wrappedDefaultOf } from '../../src/exports.js'

describe('wrappedDefaultOf', () => {
  it.each([
    ['export default React.forwardRef(InternalTable) as RefInternalTable', 'InternalTable'],
    ['export default memo(forwardRef<HTMLDivElement, Props>(Card))', 'Card'],
    ['export default memo(Card)', 'Card'],
  ])('reads the local a call wraps in %s', (source, expected) => {
    expect(wrappedDefaultOf(source)).toBe(expected)
  })

  it('says nothing when no call wraps a local', () => {
    expect(wrappedDefaultOf('export default makePanel()')).toBeUndefined()
    expect(wrappedDefaultOf('export default { a: 1 }')).toBeUndefined()
    expect(wrappedDefaultOf('export default Card')).toBeUndefined()
    expect(wrappedDefaultOf('export { Inner as default }')).toBeUndefined()
    expect(wrappedDefaultOf('export const Card = () => null')).toBeUndefined()
  })
})
