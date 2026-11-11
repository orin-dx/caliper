// What a module's source says its default export stands for, read only when the session records no component for it.

/**
 * The local name a call wraps in the default export: `export default React.forwardRef(Card) as T` and
 * `export default memo(forwardRef<HTMLDivElement, Props>(Card))` give `Card`. The extractor records no export for a
 * component under such a call. Undefined when the source has none.
 */
export function wrappedDefaultOf(source: string): string | undefined {
  return /export\s+default\s+(?:[\w.]+\s*(?:<[^>(]*>)?\(\s*)+(\w+)/.exec(source)?.[1]
}
