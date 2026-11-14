import type { Shared, SharedTagged } from './Shared'

interface FromBase extends Shared { /** Own. */ own?: string }

export function Eight(props: FromBase) {
  return null
}
export function Nine(props: Omit<SharedTagged, 'a'> & Pick<Shared, 'id'>) {
  return null
}
export function Ten(props: Shared) {
  return null
}
export function Eleven(props: SharedTagged) {
  return null
}
