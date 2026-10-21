---
cargo/caliper-docgen-core: patch
cargo/caliper-docgen: patch
cargo/caliper-docgen-napi: patch
npm/caliper-docgen: patch
npm/caliper-docgen-napi: patch
npm/caliper-docgen-vite-plugin: patch
---

- **Default exports:** anonymous default-exported components are detected and named after their file: `export default (props: P) => …`, `export default memo(function Badge(…))`, `export default forwardRef<El, P>(…)` and `export default class extends Component<P>`.
- **Wrapper types:** `Required<P>` as a props type marks P's props required, and `PropsWithChildren<P>` adds a `children` prop.
- **Excludes:** the built-in `.test.`, `.spec.`, `.stories.`, `__snapshots__` and `node_modules` checks, and user excludes, now apply to the path inside each source directory. Before, a project checked out under a directory such as `my.test.repo` produced no components and no warning.
