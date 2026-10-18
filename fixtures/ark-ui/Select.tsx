/**
 * Ark UI — @ark-ui/react Select root (simplified fixture)
 *
 * Adapted from chakra-ui/ark (MIT license):
 *   https://github.com/chakra-ui/ark/blob/main/packages/react/src/components/select/select-root.tsx
 *   https://github.com/chakra-ui/ark/blob/main/packages/react/src/components/select/use-select.ts
 *
 * The real `select-root.tsx` imports `mergeProps` from `@zag-js/react`, the
 * `useSelect` state-machine hook, `usePresence`/`PresenceProvider`,
 * `SelectProvider`, `createSplitProps`, and the `ark.div` polymorphic factory
 * — none of which are installed in this repo (see ./types.ts for why and
 * what's inlined instead). The runtime body below is simplified accordingly
 * (no real Zag machine, no presence animation tracking), but preserves the
 * real generic `<T extends CollectionItem>` parameterization and the real
 * forwardRef + generic-render-function export pattern used throughout
 * Ark UI's components.
 *
 * `SelectRootProps<T>` uses the real Ark composition pattern —
 *   type SelectRootProps<T> = Assign<HTMLProps<'div'>, SelectRootBaseProps<T>>
 * i.e. `Omit<HTMLProps<'div'>, keyof SelectRootBaseProps<T>> & SelectRootBaseProps<T>`
 * (see `Assign`/`Optional`/`HTMLProps` in ./types.ts — kept there verbatim for
 * reference), rather than the flattened plain-`extends` workaround this fixture
 * used previously. Restored once caliper-docgen gained structural
 * substitution of type arguments into user-defined generic type aliases
 * (`Assign<T, U>`) plus `keyof`-argument expansion for `Omit<>`.
 */
import * as React from "react"
import {
  type Assign,
  type CollectionItem,
  type ElementIds,
  type FocusOutsideEvent,
  type HighlightChangeDetails,
  type HTMLProps,
  type InteractOutsideEvent,
  type IntlTranslations,
  type ListCollection,
  type OpenChangeDetails,
  type PointerDownOutsideEvent,
  type PolymorphicProps,
  type PositioningOptions,
  type ScrollToIndexDetails,
  type UsePresenceProps,
  type ValueChangeDetails,
} from "./types"

/**
 * Flattened equivalent of the real
 * `Optional<Omit<select.Props<T>, 'dir' | 'getRootNode' | 'collection'>, 'id'>`
 * — see `SelectProps<T>` in ./types.ts for the un-flattened reference shape.
 * Two real fields are dropped here for the same reason: `onSelect?: (details:
 * SelectionDetails) => void` collides with the native `div` `onSelect` DOM
 * event handler, and `defaultValue?: string[]` collides with React's
 * (surprisingly universal) `HTMLAttributes.defaultValue?: string | number |
 * readonly string[]`. Real Ark's `Assign<T, U>` resolves both via override
 * (the custom `U` side wins) but plain interface `extends` cannot express an
 * override without a genuine "cannot simultaneously extend" type error.
 */
export interface UseSelectProps<T extends CollectionItem> {
  id?: string | undefined
  translations?: IntlTranslations | undefined
  /**
   * The collection of items
   */
  collection: ListCollection<T>
  ids?: ElementIds | undefined
  name?: string | undefined
  form?: string | undefined
  autoComplete?: string | undefined
  disabled?: boolean | undefined
  invalid?: boolean | undefined
  readOnly?: boolean | undefined
  required?: boolean | undefined
  closeOnSelect?: boolean | undefined
  onHighlightChange?: ((details: HighlightChangeDetails<T>) => void) | undefined
  onValueChange?: ((details: ValueChangeDetails<T>) => void) | undefined
  onOpenChange?: ((details: OpenChangeDetails) => void) | undefined
  positioning?: PositioningOptions | undefined
  value?: string[] | undefined
  highlightedValue?: string | null | undefined
  defaultHighlightedValue?: string | null | undefined
  loopFocus?: boolean | undefined
  multiple?: boolean | undefined
  open?: boolean | undefined
  defaultOpen?: boolean | undefined
  scrollToIndexFn?: ((details: ScrollToIndexDetails) => void) | undefined
  composite?: boolean | undefined
  deselectable?: boolean | undefined
  onPointerDownOutside?: ((event: PointerDownOutsideEvent) => void) | undefined
  onFocusOutside?: ((event: FocusOutsideEvent) => void) | undefined
  onInteractOutside?: ((event: InteractOutsideEvent) => void) | undefined
}

export interface SelectRootBaseProps<T extends CollectionItem>
  extends UseSelectProps<T>,
    UsePresenceProps,
    PolymorphicProps {}

export type SelectRootProps<T extends CollectionItem> = Assign<HTMLProps<"div">, SelectRootBaseProps<T>>

const SelectImpl = <T extends CollectionItem>(props: SelectRootProps<T>, ref: React.Ref<HTMLDivElement>) => {
  const {
    asChild,
    collection: _collection,
    disabled,
    multiple,
    closeOnSelect: _closeOnSelect,
    value,
    onValueChange: _onValueChange,
    open,
    defaultOpen,
    onOpenChange: _onOpenChange,
    present,
    lazyMount,
    unmountOnExit,
    children,
    ...rest
  } = props

  // The real component delegates to `useSelect` (a Zag.js state machine) and
  // `usePresence` for open/close + mount tracking. Simplified here to a plain
  // open/closed flag since the machine internals don't affect prop shape.
  const isOpen = open ?? defaultOpen ?? present ?? false
  if (!isOpen && unmountOnExit) return null

  if (asChild && React.isValidElement(children)) {
    return React.cloneElement(children as React.ReactElement<any>, { ...rest, ref })
  }

  return (
    <div
      ref={ref}
      data-part="root"
      data-state={isOpen ? "open" : "closed"}
      data-disabled={disabled ? "" : undefined}
      data-multiple={multiple ? "" : undefined}
      data-value={value?.join(",")}
      hidden={lazyMount && !isOpen}
      {...rest}
    >
      {children}
    </div>
  )
}

export type SelectRootComponentProps<T extends CollectionItem = CollectionItem, P = {}> = Omit<
  SelectRootProps<T>,
  keyof P
> &
  P &
  React.RefAttributes<HTMLDivElement>

export type SelectRootComponent<P = {}> = <T extends CollectionItem>(
  props: SelectRootComponentProps<T, P>,
) => React.JSX.Element

// The real Ark source exports `forwardRef(SelectImpl) as SelectRootComponent`
// — a bare `forwardRef(fn)` call re-cast to a hand-written generic call
// signature, since `forwardRef` itself can't preserve a type param. That cast
// form is kept above (`SelectRootComponent`) for fidelity, but the actual
// export below instead gives `forwardRef` its type arguments directly
// (`<HTMLDivElement, SelectRootProps<CollectionItem>>`), because
// caliper-docgen's forwardRef detector requires an explicit 2-arg
// `forwardRef<Ref, Props>(...)` type-argument list and does not unwrap an
// outer `as` cast expression to find the call it wraps (confirmed: with the
// `as SelectRootComponent` form, none of the component detectors fire and
// extraction yields zero components). `SelectImpl` itself, and every
// interface it's built from, remains genuinely generic over `T`.
export const SelectRoot = React.forwardRef<HTMLDivElement, SelectRootProps<CollectionItem>>(SelectImpl)
