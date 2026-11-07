// A literal Norm for tests: the kind is the one the text spells unless the test says otherwise, so `lit('1')` is the
// number `1` and `lit('1', 'string')` is the string `"1"`.
import type { Norm } from '../src/oracle-compare.ts'

type Lit = Extract<Norm, { k: 'lit' }>

const spelledKind = (value: string): Lit['of'] => {
  if (value === 'true' || value === 'false') return 'boolean'
  return /^-?\d+(\.\d+)?$/.test(value) ? 'number' : 'string'
}

export const lit = (value: string, of: Lit['of'] = spelledKind(value)): Norm => ({ k: 'lit', value, of })
