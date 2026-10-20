# caliper-docgen-vite-plugin

Vite plugin for [caliper-docgen](https://github.com/orin-dx/caliper): React component prop metadata on a virtual module, updated through HMR as components change.

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

Extraction runs on OXC through [`caliper-docgen-napi`](https://www.npmjs.com/package/caliper-docgen-napi): about 50ms cold across 21 real component libraries and about 2ms for an incremental single-file update. Requires Vite 7 or 8.

Options, benchmarks and migration notes: https://github.com/orin-dx/caliper

License: AGPL-3.0-or-later.
