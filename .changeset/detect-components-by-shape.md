---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Components are now found by the shape of the code instead of the names it uses: `memo(forwardRef(function Button(props: ButtonProps, ref) {…}))` with no type arguments is a component with a `ref`, React's `forwardRef`, `memo` and `createContext` are recognized under any import alias or namespace, and a tagged factory such as `chakra("div", {…})` takes the props type declared beside it that gives its tag (`interface CenterProps extends HTMLChakraProps<"div">`) rather than whichever type is named `{Name}Props`. A props type another component already takes is never reused, and a tag-only factory whose file declares no matching props type is skipped with a diagnostic.
