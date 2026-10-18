import type { ComponentProps } from 'react';
import { forwardRef } from 'react';

import { isPropValid, styled } from './theming';

/**
 * Storybook — code/core/src/components/components/Button/Button.tsx (simplified fixture)
 *
 * Adapted from the real source, MIT licensed:
 * https://github.com/storybookjs/storybook/blob/next/code/core/src/components/components/Button/Button.tsx
 * (`styled` re-export: code/core/src/theming/index.ts, stubbed here as ./theming.ts)
 *
 * This covers a *different* generic shape than the zendesk-garden fixture's
 * styled-components `.attrs<T>()` pattern (neither our tool nor real RDT
 * detects that one — a shared blind spot). Here, `@emotion/styled`'s
 * `styled(tag, options)` two-argument overload is used with
 * `shouldForwardProp`, and the props generic is applied to the *curried
 * call result* of `styled(...)`, not to a chained `.attrs()` method:
 *
 *   const StyledButton = styled('button', { shouldForwardProp: isPropValid })<{
 *     size?: 'small' | 'medium';
 *     ...
 *   }>(({ theme, ... }) => ({ ... }));
 *
 * `Button`'s own props type then threads that shape back out via a utility
 * type over the styled component itself: `ButtonProps extends
 * Omit<ComponentProps<typeof StyledButton>, 'as'>` — real RDT/caliper-docgen
 * would need to resolve `typeof StyledButton`'s inferred return type through
 * `@emotion/styled`'s overloaded `CreateStyled` call signature to recover
 * those props, which is a materially different (and arguably harder) chain
 * than resolving a single `.attrs<T>()` type argument.
 *
 * Trimmed/stubbed for this fixture:
 *   - `storybook/internal/client-logger`'s `deprecate(...)` calls (deprecation
 *     warnings for `ariaLabel`/`active`) are dropped — no effect on prop shape.
 *   - `@radix-ui/react-slot`'s `Slot` / `asChild` polymorphic-composition
 *     wiring is dropped; `asChild` remains as a real, declared boolean prop
 *     (matching upstream) but is unused in the trimmed body.
 *   - `storybook/manager-api`'s `shortcutToAriaKeyshortcuts` helper and the
 *     real `API_KeyCollection` type are dropped; a minimal local type stand-in
 *     is inlined below since only the prop's *type* matters here.
 *   - `InteractiveTooltipWrapper` / `useAriaDescription` (tooltip and
 *     aria-describedby wiring, plus their own component/hook files) and the
 *     animation-timeout `useEffect`/`useState` are all removed — internal
 *     behavior, irrelevant to prop extraction.
 *   - The real ~150-line theme-driven `CSSObject` body (colors, hover/active/
 *     focus-visible states, animation keyframes) is collapsed to a trivial
 *     stand-in, consistent with how the zendesk-garden fixture collapses its
 *     style functions.
 *   - `IconButton` (a trivial deprecated `forwardRef` wrapper around `Button`
 *     that just spreads props through) is dropped — it adds no new
 *     prop-extraction signal beyond `Button` itself.
 */

/** Simplified stand-in for the real `API_KeyCollection` from `storybook/manager-api`. */
type API_KeyCollection = string[];

export interface ButtonProps extends Omit<ComponentProps<typeof StyledButton>, 'as'> {
  as?: ComponentProps<typeof StyledButton>['as'];
  asChild?: boolean;

  /**
   * A concise action label for the button announced by screen readers. Needed for buttons without
   * text or with text that relies on visual cues to be understood. Pass false to indicate that the
   * Button's content is already accessible to all. When a string is passed, it is also used as the
   * default tooltip text.
   */
  ariaLabel?: string | false;

  /**
   * An optional tooltip to display when the Button is hovered. If the Button has no text content,
   * consider making this the same as the aria-label.
   */
  tooltip?: string;

  /**
   * Only use this flag when tooltips on button interfere with other keyboard interactions, like
   * when building a custom select or menu button. Disables tooltips from the `tooltip`, `shortcut`
   * and `ariaLabel` props.
   */
  disableAllTooltips?: boolean;

  /**
   * A more thorough description of what the Button does, provided to non-sighted users through an
   * aria-describedby attribute. Use sparingly for buttons that trigger complex actions.
   */
  ariaDescription?: string;

  /**
   * An optional keyboard shortcut to enable the button. Will be displayed in the tooltip and passed
   * to aria-keyshortcuts for assistive technologies. The binding of the shortcut and action is
   * managed globally in the manager's shortcuts module.
   */
  shortcut?: API_KeyCollection;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      as = 'button',
      asChild = false,
      animation = 'none',
      size = 'small',
      appearance = 'default',
      variant = 'outline',
      padding = 'medium',
      disabled = false,
      readOnly = false,
      active,
      onClick,
      ariaLabel,
      ariaDescription = undefined,
      tooltip = undefined,
      shortcut = undefined,
      disableAllTooltips = false,
      ...props
    },
    ref
  ) => {
    // Trimmed: deprecation warnings, tooltip/aria-description wiring, and the
    // animation-timeout effect are all removed — irrelevant to prop shape.
    return (
      <StyledButton
        as={as}
        ref={ref}
        appearance={appearance}
        variant={variant}
        size={size}
        padding={padding}
        $disabled={disabled || readOnly}
        aria-disabled={disabled || readOnly ? 'true' : undefined}
        readOnly={readOnly}
        active={active}
        animation={animation}
        onClick={disabled || readOnly ? undefined : onClick}
        aria-label={!readOnly && ariaLabel !== false ? ariaLabel : undefined}
        {...props}
      />
    );
  }
);

Button.displayName = 'Button';

const StyledButton = styled('button', {
  shouldForwardProp: (prop) => isPropValid(prop),
})<{
  size?: 'small' | 'medium';
  padding?: 'small' | 'medium' | 'none';
  appearance?: 'default' | 'agentic';
  variant?: 'outline' | 'solid' | 'ghost';
  active?: boolean;
  $disabled?: boolean;
  readOnly?: boolean;
  animating?: boolean;
  animation?: 'none' | 'rotate360' | 'glow' | 'jiggle';
}>((props) => {
  // Trimmed: the real implementation branches on appearance/variant/size/
  // padding/$disabled/readOnly/active/animating against `theme` to compute
  // ~150 lines of colors, hover/active/focus-visible states, and animation
  // keyframes. Collapsed to a stub — not relevant to prop extraction.
  return {
    border: 0,
    display: 'inline-flex',
    cursor: props.readOnly ? 'inherit' : props.$disabled ? 'not-allowed' : 'pointer',
  };
});
