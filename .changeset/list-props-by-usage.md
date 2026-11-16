---
cargo/caliper-docgen-core: minor
cargo/caliper-docgen: minor
cargo/caliper-docgen-napi: minor
npm/caliper-docgen: minor
npm/caliper-docgen-storybook: minor
npm/caliper-docgen-napi: minor
npm/caliper-docgen-vite-plugin: minor
---

Storybook Controls and the CLI now list a component's main props in full, plus the props the component itself uses, and leave large generated sets as names. Every prop of the component's props type and of the small interfaces it inherits from (Chakra's `loadingText`, `spinner`, `asChild`, `as`, `unstyled`, `css`, `variant`, `size`, the `html*` props) is a row, whether or not a story sets it. A DOM or ARIA attribute, or a style-system prop, becomes a row when the component's body reads it, destructures it or passes it on by name; spreading the props onto a DOM tag does not count. The hundreds of `@types/react` attributes and generated style props (Chakra's `SystemProperties` and conditions such as `_hover`) stay as names in their inheritance layer. A button over Chakra UI's `ButtonProps` goes from 437 Controls rows to 21, and a plain `ComponentProps<'span'>` badge from 280 to its 2 own props. React Aria and React Spectrum components list their documented props again (`id`, `aria-label`, `aria-labelledby`, `aria-describedby`, `aria-details`, `excludeFromTabOrder`, `routerOptions`, and props such as `onFocus` that a React Aria interface declares), while the global DOM events stay names; in Chakra UI's own repository a `Button` lists 17 props instead of 147, leaving the 130 style conditions as names. `htmlAttributes` still says which DOM attributes are known, not how many become rows; `collapseInheritedProps: 'none'` lists every resolved prop as before, including the React props of a component whose props type the checker alone can read (`ReturnType<typeof makeProps>`).
