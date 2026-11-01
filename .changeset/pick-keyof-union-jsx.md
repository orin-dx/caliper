---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Props built from React's element attributes keep them in more places: `Pick<DetailedHTMLProps<…>, keyof React.HTMLAttributes<T> | "value">`, a union of props interfaces (bare or inside `Omit`), next/image's global `JSX.IntrinsicElements["img"]` and a generic alias bound to a union of tags (`ComponentPropsWithRef<"h1" | "h2">`) now list the element's attributes as `React.JSX.IntrinsicElements` does, in every `htmlAttributes` mode. For a union of tags, `htmlAttributes: curated` lists each tag's notable attributes together, so a name two tags share is listed once, as the first tag in the union declares it.
