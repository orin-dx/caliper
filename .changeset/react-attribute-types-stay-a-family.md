---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

With `--experimental-semantic-types`, a prop typed `Omit<React.ComponentPropsWithoutRef<"a">, "href">`, or an interface that extends `Omit<React.HTMLAttributes<HTMLDivElement>, "color">`, no longer comes back cut off as `truncated`: React's DOM attribute types are kept as a family (`htmlAttributes` for the element) instead of expanded into hundreds of props, and an interface you declare is always listed in full however many fields it has, with the types it extends named beside them. `ReactNode | ((open: boolean) => ReactNode)` is `ReactNode` plus the function again, and a wide `ElementType` in a nested prop stays `ElementType`. The checker is still off by default.
