/**
 * Notices when a lazy session is being read across the whole project. One component resolved alone costs several times
 * what the same component costs in a pass over all of them, so after `after` distinct components the session resolves
 * the rest together, once. `0` never does.
 */
export class Fill {
  private readonly after: number
  private readonly requested = new Set<string>()
  private filling = false

  constructor(after: number) {
    this.after = after
  }

  /** Records that `key` was asked for; true exactly once, when the rest should be resolved now. */
  asked(key: string): boolean {
    this.requested.add(key)
    if (this.filling || this.after <= 0 || this.requested.size < this.after) return false
    this.filling = true
    return true
  }
}
