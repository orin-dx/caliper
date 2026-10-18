# Phase 5a — Vite Plugin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure the monorepo and implement `@oxc-react-docgen/vite-plugin` — a single Vite 7+ plugin that runs incremental prop extraction via the native NAPI binary and broadcasts updates over HMR.

**Architecture:** The NAPI crate moves from `crates/napi/` to `crates/binding/` (a name that reflects its role as a thin binding layer). `packages/napi/` becomes a real npm package with a dev `index.js` loader. The vite plugin lives in `packages/vite-plugin/` and wraps the NAPI session lifecycle around Vite's `configResolved` → `configureServer` → `hotUpdate` → `buildEnd` hooks.

**Tech Stack:** Rust/NAPI-RS 3, TypeScript 5, Vite 7, Vitest 3, pnpm workspaces, Moon 2

---

## File map

```
# Deleted
packages/rolldown-plugin/           ← entire directory removed

# Renamed (directory only; Rust package name unchanged)
crates/napi/         → crates/binding/

# Moved
packages/validate/   → apps/validate/

# Modified
Cargo.toml                          ← workspace members
.moon/workspace.yml                 ← project paths

# New files — packages/napi/
packages/napi/package.json          ← @oxc-react-docgen/napi npm manifest
packages/napi/index.js              ← dev binary loader (CJS, loads .node from target/)
packages/napi/tsconfig.json

# New files — packages/vite-plugin/
packages/vite-plugin/package.json   ← @oxc-react-docgen/vite-plugin npm manifest
packages/vite-plugin/tsconfig.json
packages/vite-plugin/vitest.config.ts
packages/vite-plugin/src/index.ts   ← full plugin implementation
packages/vite-plugin/tests/unit/plugin.test.ts
```

---

## Task 1: Delete rolldown-plugin and clean workspace config

**Files:**

- Delete: `packages/rolldown-plugin/` (whole directory)
- Modify: `Cargo.toml`
- Modify: `.moon/workspace.yml`

- [ ] **Step 1: Remove the directory**

```bash
rm -rf packages/rolldown-plugin
```

- [ ] **Step 2: Remove from Cargo workspace**

In `Cargo.toml`, change:

```toml
[workspace]
members = [
    "crates/core",
    "crates/napi",
    "crates/cli",
    "packages/rolldown-plugin",
]
```

To:

```toml
[workspace]
members = [
    "crates/core",
    "crates/napi",
    "crates/cli",
]
```

- [ ] **Step 3: Remove from Moon workspace**

In `.moon/workspace.yml`, remove the `rolldown-plugin` line:

```yaml
projects:
  core: 'crates/core'
  cli: 'crates/cli'
  crates-napi: 'crates/napi'
  napi: 'packages/napi'
  validate: 'packages/validate'
  vite-plugin: 'packages/vite-plugin'
```

- [ ] **Step 4: Verify build still passes**

```bash
cargo check --workspace
```

Expected: clean (no errors referencing rolldown-plugin).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock .moon/workspace.yml
git commit -m "chore: delete packages/rolldown-plugin — Rolldown native Rust plugin not viable"
```

---

## Task 2: Rename crates/napi → crates/binding

The directory name `crates/napi/` was misleading — it's a binding crate, not the npm package. The Rust package name (`oxc-react-docgen-napi`) stays the same so the `.node` binary filename doesn't change.

**Files:**

- Rename: `crates/napi/` → `crates/binding/`
- Modify: `Cargo.toml` (workspace member path)
- Modify: `.moon/workspace.yml` (project path)

- [ ] **Step 1: Move the directory**

```bash
mv crates/napi crates/binding
```

- [ ] **Step 2: Update workspace member path in Cargo.toml**

Change:

```toml
members = [
    "crates/core",
    "crates/napi",
    "crates/cli",
]
```

To:

```toml
members = [
    "crates/core",
    "crates/binding",
    "crates/cli",
]
```

The path inside `crates/binding/Cargo.toml` has `oxc-react-docgen-core = { path = "../core" }` — this relative path is still correct after the rename, no change needed there.

- [ ] **Step 3: Update Moon workspace**

In `.moon/workspace.yml`, change:

```yaml
crates-napi: 'crates/napi'
```

To:

```yaml
crates-binding: 'crates/binding'
```

- [ ] **Step 4: Verify build**

```bash
cargo check --workspace
cargo test -p oxc-react-docgen-core 2>&1 | tail -5
```

Expected: 100 tests pass, no errors.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock .moon/workspace.yml crates/binding
git commit -m "chore: rename crates/napi → crates/binding (Rust package name unchanged)"
```

---

## Task 3: Move packages/validate → apps/validate

`pnpm-workspace.yaml` already includes `apps/*`, so pnpm picks this up automatically.

**Files:**

- Create: `apps/` directory
- Move: `packages/validate/` → `apps/validate/`
- Modify: `.moon/workspace.yml`

- [ ] **Step 1: Create apps/ and move**

```bash
mkdir -p apps
mv packages/validate apps/validate
```

- [ ] **Step 2: Update Moon workspace**

In `.moon/workspace.yml`, change:

```yaml
validate: 'packages/validate'
```

To:

```yaml
validate: 'apps/validate'
```

- [ ] **Step 3: Verify pnpm still resolves the workspace**

```bash
pnpm install
```

Expected: clean install, no "workspace package not found" errors.

- [ ] **Step 4: Commit**

```bash
git add apps/validate .moon/workspace.yml
git rm -r packages/validate  # if git doesn't auto-detect the move
git commit -m "chore: move packages/validate → apps/validate"
```

---

## Task 4: Set up packages/napi as an npm package

`packages/napi/` currently has only `index.d.ts`. It needs a `package.json`, a dev `index.js` loader that finds the native binary from the Cargo build output, and a `tsconfig.json`.

**Files:**

- Create: `packages/napi/package.json`
- Create: `packages/napi/index.js`
- Create: `packages/napi/tsconfig.json`

- [ ] **Step 1: Write package.json**

Create `packages/napi/package.json`:

```json
{
  "name": "@oxc-react-docgen/napi",
  "version": "0.1.0",
  "description": "Native Node.js bindings for oxc-react-docgen",
  "main": "index.js",
  "types": "index.d.ts",
  "type": "commonjs",
  "napi": {
    "binaryName": "oxc_react_docgen_napi",
    "targets": [
      "aarch64-apple-darwin",
      "x86_64-apple-darwin",
      "x86_64-unknown-linux-gnu",
      "x86_64-pc-windows-msvc",
      "aarch64-unknown-linux-gnu"
    ]
  },
  "files": ["index.js", "index.d.ts", "*.node"],
  "license": "MIT"
}
```

- [ ] **Step 2: Write the dev binary loader**

Create `packages/napi/index.js`:

```js
'use strict'

const { existsSync } = require('node:fs')
const { join } = require('node:path')

if (process.env.NAPI_RS_NATIVE_LIBRARY_PATH) {
  module.exports = require(process.env.NAPI_RS_NATIVE_LIBRARY_PATH)
} else {
  const binaryName = 'oxc_react_docgen_napi'
  const candidates = [
    join(__dirname, `${binaryName}.node`),
    join(__dirname, '../../target/release', `${binaryName}.node`),
    join(__dirname, '../../target/debug', `${binaryName}.node`),
  ]
  const found = candidates.find((p) => existsSync(p))
  if (!found) {
    throw new Error(
      `@oxc-react-docgen/napi: native binary not found.\n` +
        `Run: cargo build -p oxc-react-docgen-napi\n` +
        `Searched:\n${candidates.map((p) => `  ${p}`).join('\n')}`,
    )
  }
  module.exports = require(found)
}
```

- [ ] **Step 3: Write tsconfig.json**

Create `packages/napi/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "CommonJS",
    "strict": true,
    "skipLibCheck": true
  },
  "include": ["index.d.ts"]
}
```

- [ ] **Step 4: Verify pnpm resolves the package**

```bash
pnpm install
node -e "require('@oxc-react-docgen/napi')" 2>&1
```

Expected output (binary not built yet):

```
Error: @oxc-react-docgen/napi: native binary not found.
Run: cargo build -p oxc-react-docgen-napi
```

This error is correct — it means the loader ran and gave a clear message.

- [ ] **Step 5: Commit**

```bash
git add packages/napi/package.json packages/napi/index.js packages/napi/tsconfig.json
git commit -m "feat: set up packages/napi as npm package with dev binary loader"
```

---

## Task 5: Scaffold packages/vite-plugin

**Files:**

- Create: `packages/vite-plugin/package.json`
- Create: `packages/vite-plugin/tsconfig.json`
- Create: `packages/vite-plugin/vitest.config.ts`
- Create: `packages/vite-plugin/src/index.ts` (stub only — implementation in Task 7)

- [ ] **Step 1: Write package.json**

Create `packages/vite-plugin/package.json`:

```json
{
  "name": "@oxc-react-docgen/vite-plugin",
  "version": "0.1.0",
  "description": "Vite plugin for oxc-react-docgen — live prop type extraction",
  "type": "module",
  "main": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": {
      "import": "./dist/index.js",
      "types": "./dist/index.d.ts"
    }
  },
  "scripts": {
    "build": "tsc",
    "test": "vitest run"
  },
  "peerDependencies": {
    "vite": "^7.0.0 || ^8.0.0"
  },
  "dependencies": {
    "@oxc-react-docgen/napi": "workspace:*"
  },
  "devDependencies": {
    "vite": "^7.0.0",
    "vitest": "^3.0.0",
    "typescript": "^5.8.0",
    "@types/node": "^22.0.0"
  },
  "license": "MIT"
}
```

- [ ] **Step 2: Write tsconfig.json**

Create `packages/vite-plugin/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "declaration": true,
    "outDir": "./dist",
    "rootDir": "./src",
    "skipLibCheck": true
  },
  "include": ["src/**/*"]
}
```

- [ ] **Step 3: Write vitest.config.ts**

Create `packages/vite-plugin/vitest.config.ts`:

```typescript
import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    pool: 'threads',
    include: ['tests/unit/**/*.test.ts'],
  },
})
```

- [ ] **Step 4: Write the stub src/index.ts**

Create `packages/vite-plugin/src/index.ts`:

```typescript
import type { Plugin } from 'vite'

export interface OxcDocgenOptions {
  srcDirs: string[]
  exclude?: string[]
  reactVersion?: 'react18' | 'react19'
  skipHtmlProps?: boolean
}

export function oxcReactDocgen(_options: OxcDocgenOptions): Plugin {
  return { name: 'oxc-react-docgen' }
}
```

- [ ] **Step 5: Install deps**

```bash
pnpm install
```

Expected: `@oxc-react-docgen/napi` and `vite` resolved from workspace/registry.

- [ ] **Step 6: Commit**

```bash
git add packages/vite-plugin/
git commit -m "chore: scaffold packages/vite-plugin with package.json, tsconfig, vitest config"
```

---

## Task 6: Write failing unit tests for the vite plugin

TDD: write all tests against the stub — they should fail. The tests use `vi.mock` so no native binary is needed.

**Files:**

- Create: `packages/vite-plugin/tests/unit/plugin.test.ts`

- [ ] **Step 1: Create the test file**

Create `packages/vite-plugin/tests/unit/plugin.test.ts`:

```typescript
import { describe, it, expect, vi, beforeEach } from 'vitest'

// Mock BEFORE importing plugin — vi.mock is hoisted by Vitest
vi.mock('@oxc-react-docgen/napi', () => ({
  createSession: vi.fn().mockReturnValue(42),
  initializeSession: vi
    .fn()
    .mockResolvedValue(JSON.stringify({ components: {}, enums: {}, diagnostics: [], stats: {} })),
  extractFileIncremental: vi.fn().mockResolvedValue(
    JSON.stringify({
      updatedComponents: [
        {
          displayName: 'Button',
          filePath: '/project/src/Button.tsx',
          props: {},
          inheritance: [],
          notableInherited: {},
          description: '',
          discriminantProp: null,
          composes: [],
          tags: {},
          methods: [],
        },
      ],
      affectedFiles: ['/project/src/Button.tsx'],
      diagnostics: [],
      durationMs: 5,
    }),
  ),
  closeSession: vi.fn(),
}))

import { oxcReactDocgen } from '../../src/index.js'
import * as napi from '@oxc-react-docgen/napi'

describe('oxcReactDocgen', () => {
  let plugin: ReturnType<typeof oxcReactDocgen>

  beforeEach(() => {
    vi.clearAllMocks()
    plugin = oxcReactDocgen({ srcDirs: ['src'] })
    // Simulate Vite calling configResolved
    ;(plugin.configResolved as Function)({ root: '/project' })
  })

  // ── resolveId ──────────────────────────────────────────────────────────────

  describe('resolveId', () => {
    it('resolves the virtual module id', () => {
      const result = (plugin.resolveId as Function)('virtual:oxc-react-docgen')
      expect(result).toBe('\0virtual:oxc-react-docgen')
    })

    it('returns undefined for unrelated ids', () => {
      expect((plugin.resolveId as Function)('react')).toBeUndefined()
      expect((plugin.resolveId as Function)('./Button.tsx')).toBeUndefined()
    })
  })

  // ── load ───────────────────────────────────────────────────────────────────

  describe('load', () => {
    it('returns a JS module for the resolved virtual id', () => {
      const code = (plugin.load as Function)('\0virtual:oxc-react-docgen')
      expect(code).toContain('export default')
      expect(code).toContain('"components"')
    })

    it('returns undefined for other ids', () => {
      expect((plugin.load as Function)('\0other')).toBeUndefined()
      expect((plugin.load as Function)('virtual:oxc-react-docgen')).toBeUndefined()
    })
  })

  // ── configResolved ─────────────────────────────────────────────────────────

  describe('configResolved', () => {
    it('creates a NAPI session with resolved absolute src dirs', () => {
      expect(napi.createSession).toHaveBeenCalledWith(expect.objectContaining({ srcDirs: ['/project/src'] }))
    })
  })

  // ── hotUpdate ──────────────────────────────────────────────────────────────

  describe('hotUpdate', () => {
    const clientEnv = () => ({
      environment: { name: 'client', hot: { send: vi.fn() } },
    })

    const makeOpts = (file: string) => ({
      file,
      type: 'update' as const,
      modules: [],
      timestamp: 1000,
      read: vi.fn().mockResolvedValue(''),
      server: {} as any,
    })

    it('calls extractFileIncremental for tsx files in srcDirs', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(napi.extractFileIncremental).toHaveBeenCalledWith(
        '/project/src/Button.tsx',
        42,
        expect.objectContaining({ srcDirs: ['/project/src'] }),
      )
    })

    it('sends oxc-react-docgen:update over HMR', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(ctx.environment.hot.send).toHaveBeenCalledWith(
        'oxc-react-docgen:update',
        expect.objectContaining({ file: '/project/src/Button.tsx' }),
      )
    })

    it('returns undefined so React Fast Refresh still processes the file', async () => {
      const ctx = clientEnv()
      const result = await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(result).toBeUndefined()
    })

    it('skips SSR and other non-client environments', async () => {
      // The implementation reads `(this as any).environment` — ctx IS the `this` value
      const ctx = { environment: { name: 'ssr', hot: { send: vi.fn() } } }
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })

    it('skips files outside srcDirs', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/node_modules/react/index.js'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })

    it('skips non-ts/tsx files inside srcDirs', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/styles.css'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })
  })

  // ── buildEnd ───────────────────────────────────────────────────────────────

  describe('buildEnd', () => {
    it('closes the NAPI session', () => {
      ;(plugin.buildEnd as Function)()
      expect(napi.closeSession).toHaveBeenCalledWith(42)
    })
  })
})
```

- [ ] **Step 2: Run the tests — they should fail**

```bash
cd packages/vite-plugin && pnpm test
```

Expected: failures like `resolveId returned undefined` (stub returns `{ name: 'oxc-react-docgen' }` only).

- [ ] **Step 3: Commit the failing tests**

```bash
git add packages/vite-plugin/tests/
git commit -m "test: add failing unit tests for vite plugin hooks"
```

---

## Task 7: Implement the vite plugin

Replace the stub with the full implementation. Run the tests after each hook group to verify incrementally.

**Files:**

- Modify: `packages/vite-plugin/src/index.ts`

- [ ] **Step 1: Write the full implementation**

Replace all of `packages/vite-plugin/src/index.ts`:

```typescript
import type { Plugin, ResolvedConfig, ViteDevServer, HotUpdateOptions } from 'vite'
import * as napi from '@oxc-react-docgen/napi'
import type { ExtractionOutput, IncrementalUpdate } from '@oxc-react-docgen/napi'

const VIRTUAL_ID = 'virtual:oxc-react-docgen'
const RESOLVED_VIRTUAL_ID = '\0' + VIRTUAL_ID

export interface OxcDocgenOptions {
  srcDirs: string[]
  exclude?: string[]
  reactVersion?: 'react18' | 'react19'
  skipHtmlProps?: boolean
}

export function oxcReactDocgen(options: OxcDocgenOptions): Plugin {
  let sessionId: number
  let root: string
  let currentOutput: ExtractionOutput = {
    components: {},
    enums: {},
    diagnostics: [],
    stats: {
      componentsExtracted: 0,
      componentsSkipped: 0,
      filesParsed: 0,
      dtsCacheHits: 0,
      durationMs: 0,
      tier1Count: 0,
      tier3Count: 0,
      opaqueCount: 0,
    },
  }
  // Holds the in-flight init promise so hotUpdate can await it before first incremental call.
  let initPromise: Promise<void> | null = null

  function napiOptions() {
    return {
      srcDirs: options.srcDirs.map((d) => (d.startsWith('/') ? d : `${root}/${d}`)),
      exclude: options.exclude,
      reactVersion: options.reactVersion,
      skipHtmlProps: options.skipHtmlProps,
    }
  }

  function isSrcFile(file: string): boolean {
    const dirs = napiOptions().srcDirs
    return (file.endsWith('.tsx') || file.endsWith('.ts')) && dirs.some((dir) => file.startsWith(dir))
  }

  return {
    name: 'oxc-react-docgen',

    configResolved(config: ResolvedConfig) {
      root = config.root
      sessionId = napi.createSession(napiOptions())
    },

    configureServer(server: ViteDevServer) {
      initPromise = napi.initializeSession(sessionId, napiOptions()).then((json) => {
        currentOutput = JSON.parse(json) as ExtractionOutput
        // Notify client that initial extraction is ready
        ;(server as any).environments?.client?.hot?.send('oxc-react-docgen:ready', {
          components: currentOutput.components,
        })
      })
    },

    async hotUpdate(opts: HotUpdateOptions) {
      // Vite 6+ supplies this.environment in the hotUpdate context.
      // We cast rather than annotating `this:` to avoid fighting the Plugin interface's
      // declared this-type across different Vite patch versions.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const env = (this as any).environment as
        | { name: string; hot: { send(event: string, data?: unknown): void } }
        | undefined

      if (!env || env.name !== 'client') return
      if (!isSrcFile(opts.file)) return

      // Wait for cold extraction to finish before the first incremental call.
      if (initPromise) await initPromise

      const json = await napi.extractFileIncremental(opts.file, sessionId, napiOptions())
      const update = JSON.parse(json) as IncrementalUpdate

      for (const comp of update.updatedComponents) {
        currentOutput.components[comp.displayName] = comp
      }

      env.hot.send('oxc-react-docgen:update', {
        file: opts.file,
        updatedComponents: update.updatedComponents,
        diagnostics: update.diagnostics,
      })

      // Return undefined (NOT []) — returning [] suppresses React Fast Refresh
      // for the changed file. We want both: updated types AND component re-render.
    },

    resolveId(id: string) {
      if (id === VIRTUAL_ID) return RESOLVED_VIRTUAL_ID
    },

    load(id: string) {
      if (id !== RESOLVED_VIRTUAL_ID) return
      return `export default ${JSON.stringify(currentOutput)}`
    },

    buildEnd() {
      napi.closeSession(sessionId)
    },
  }
}
```

- [ ] **Step 2: Run the tests — all should pass**

```bash
cd packages/vite-plugin && pnpm test
```

Expected:

```
✓ resolveId > resolves the virtual module id
✓ resolveId > returns undefined for unrelated ids
✓ load > returns a JS module for the resolved virtual id
✓ load > returns undefined for other ids
✓ configResolved > creates a NAPI session with resolved absolute src dirs
✓ hotUpdate > calls extractFileIncremental for tsx files in srcDirs
✓ hotUpdate > sends oxc-react-docgen:update over HMR
✓ hotUpdate > returns undefined so React Fast Refresh still processes the file
✓ hotUpdate > skips SSR and other non-client environments
✓ hotUpdate > skips files outside srcDirs
✓ hotUpdate > skips non-ts/tsx files inside srcDirs
✓ buildEnd > closes the NAPI session
12 tests passed
```

- [ ] **Step 3: Type-check**

```bash
cd packages/vite-plugin && npx tsc --noEmit
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add packages/vite-plugin/src/index.ts
git commit -m "feat: implement @oxc-react-docgen/vite-plugin"
```

---

## Task 8: Wire Moon tasks for the new Node.js packages

Add `moon.yml` to each new package so `moon run :test` and `moon run :build` work workspace-wide.

**Files:**

- Create: `packages/napi/moon.yml`
- Create: `packages/vite-plugin/moon.yml`

- [ ] **Step 1: Write packages/napi/moon.yml**

Create `packages/napi/moon.yml`:

```yaml
language: typescript

tasks:
  build:
    command: cargo build -p oxc-react-docgen-napi
    options:
      runFromWorkspaceRoot: true
    inputs:
      - '../../crates/binding/src/**/*'
      - '../../crates/binding/Cargo.toml'
    outputs:
      - '../../target/debug/oxc_react_docgen_napi.node'
```

- [ ] **Step 2: Write packages/vite-plugin/moon.yml**

Create `packages/vite-plugin/moon.yml`:

```yaml
language: typescript

tasks:
  build:
    command: pnpm build
    inputs:
      - 'src/**/*'
      - 'tsconfig.json'
    outputs:
      - 'dist'

  test:
    command: pnpm test
    inputs:
      - 'src/**/*'
      - 'tests/**/*'
      - 'vitest.config.ts'
    deps:
      - '~:build'
```

- [ ] **Step 3: Verify Moon can run tests**

```bash
moon run vite-plugin:test
```

Expected: 12 tests pass (same as `pnpm test` from the package directory).

- [ ] **Step 4: Commit**

```bash
git add packages/napi/moon.yml packages/vite-plugin/moon.yml
git commit -m "chore: add moon.yml tasks for packages/napi and packages/vite-plugin"
```

---

## Task 9: Final smoke test

Build the native binary and verify the loader + plugin work end-to-end.

- [ ] **Step 1: Build the native binary**

```bash
cargo build -p oxc-react-docgen-napi
```

Expected: `target/debug/oxc_react_docgen_napi.node` created (macOS: `liboxc_react_docgen_napi.dylib` renamed by napi-build).

- [ ] **Step 2: Verify the loader finds the binary**

```bash
node -e "const m = require('@oxc-react-docgen/napi'); console.log(Object.keys(m))"
```

Expected:

```
[ 'extractAll', 'createSession', 'initializeSession', 'extractFileIncremental', 'closeSession' ]
```

- [ ] **Step 3: Run all Rust tests to confirm nothing regressed**

```bash
cargo test --workspace 2>&1 | tail -5
```

Expected: 100 tests pass.

- [ ] **Step 4: Run vite-plugin unit tests one more time**

```bash
cd packages/vite-plugin && pnpm test
```

Expected: 12 tests pass.

- [ ] **Step 5: Update docs/09-STATUS.md**

In the Phase table, change:

```
| 5a — Vite plugin | ❌ Not started | ...
```

To:

```
| 5a — Vite plugin | ✅ Complete | @oxc-react-docgen/vite-plugin, 12 unit tests, crates/binding rename, apps/validate move |
```

Also update **Tests** line in the header:

```
**Tests:** 112 passing, 0 failing (100 unit/snapshot Rust + 12 TS)
```

- [ ] **Step 6: Final commit**

```bash
git add docs/09-STATUS.md
git commit -m "docs: mark Phase 5a complete"
```
