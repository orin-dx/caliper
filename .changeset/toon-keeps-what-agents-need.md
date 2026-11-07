---
cargo/caliper-docgen-semantic: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
---

Agents reading TOON output no longer lose what they need to write a valid call. A literal union such as shadcn's `size` or `variant` now lists every value instead of ending in `...(+2)`, function props keep their parameter names (`(value: string, index?: number) => void`), a prop's `@deprecated`, `@see` and `@since` notes appear in a `tags` column, and a discriminated union names its discriminant in a `discriminant:` line. TOON is about 12% longer on antd and shadcn; unions of types past 8 members and intersections past 4 are still cut, with the count.
