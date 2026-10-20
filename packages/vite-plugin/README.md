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

docgen.components['Button'].props
```

Extraction runs on OXC through [`caliper-docgen-napi`](https://www.npmjs.com/package/caliper-docgen-napi), so a single-file update takes about 2ms. Requires Vite 7 or 8. The module uses the canonical output format, not the `react-docgen-typescript` shape.

Docs and benchmarks: [github.com/orin-dx/caliper](https://github.com/orin-dx/caliper).

## License

FSL-1.1-MIT. Each version becomes MIT two years after its release.
