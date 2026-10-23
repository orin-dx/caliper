# caliper-docgen-vite-plugin

A Vite plugin that exposes your React components' props on a virtual module and updates it through HMR as you edit.

```bash
npm install --save-dev caliper-docgen-vite-plugin
```

```ts
// vite.config.ts
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { caliperDocgen } from 'caliper-docgen-vite-plugin'

export default defineConfig({
  plugins: [react(), caliperDocgen({ srcDirs: ['src'] })],
})
```

```ts
import docgen from 'virtual:caliper-docgen'

// A component maps each prop name to an id; each distinct prop is written once, in `docgen.props`.
const label = docgen.props[docgen.components['Button'].props.label]
```

On every change the virtual module reloads with the same output a fresh extraction would give, including removed components, enums and diagnostics. To apply changes yourself, listen for the update event:

```ts
import.meta.hot?.on(
  'caliper-docgen:update',
  ({ updatedKeys, updatedComponents, props, removedKeys, diagnostics, removedDiagnostics }) => {
    // Events arrive in revision order, each a delta from the one before: delete removedKeys, set updatedKeys[i] to
    // updatedComponents[i], merge props into docgen.props, drop removedDiagnostics and add diagnostics. Props no
    // component refers to any more can be dropped.
  },
)
```

A change to a tsconfig, `package.json` or lockfile in or above `srcDirs` (installing a package, editing `paths`) re-extracts everything and arrives as an ordinary update event.

In `vite build --watch`, a change to any file under `srcDirs`, or to that configuration, triggers a rebuild, even one no bundle module imports. A build fails if extraction fails.

Extraction runs on OXC through [`caliper-docgen-napi`](https://www.npmjs.com/package/caliper-docgen-napi), so a single-file update takes about 2ms. Requires Vite 7 or 8. The module uses the canonical output format, not the `react-docgen-typescript` shape.

Docs and benchmarks: [orin-dx.com/tools/caliper](https://orin-dx.com/tools/caliper/). Source: [github.com/orin-dx/caliper](https://github.com/orin-dx/caliper).

## License

FSL-1.1-MIT. Each version becomes MIT two years after its release.
