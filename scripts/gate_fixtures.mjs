// Recreates the large project ADR 0026's two gates run on, outside the repository (it is hundreds of megabytes once
// installed): a wrapper component for every component `@chakra-ui/react` and `@mui/material` export, each typed as
// `ComponentProps<typeof X>`, so the checker loads both libraries' declarations for every request.
//
//   node scripts/gate_fixtures.mjs [DIR]     DIR defaults to ~/Projects/caliper-fixtures/gate-large
//
// Installs with pnpm (`--prefer-offline`, so the pnpm store is used when it has the pins) and writes `src/*.tsx`. Safe to
// rerun: it rewrites the wrappers and leaves `node_modules` alone when the install is already current.
import { execFileSync } from 'node:child_process'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { homedir } from 'node:os'
import { join, resolve } from 'node:path'

const dir = resolve(process.argv[2] ?? join(homedir(), 'Projects', 'caliper-fixtures', 'gate-large'))
const libraries = [
  { name: '@chakra-ui/react', version: '3.37.0', prefix: 'Chakra' },
  { name: '@mui/material', version: '9.5.0', prefix: 'Mui' },
]
mkdirSync(join(dir, 'src'), { recursive: true })
writeFileSync(
  join(dir, 'package.json'),
  JSON.stringify(
    {
      name: 'gate-large',
      private: true,
      type: 'module',
      dependencies: {
        ...Object.fromEntries(libraries.map(({ name, version }) => [name, version])),
        '@emotion/react': '11.14.0',
        '@emotion/styled': '11.14.1',
        react: '19.3.1',
        '@types/react': '19.3.0',
      },
    },
    null,
    2,
  ),
)
writeFileSync(
  join(dir, 'tsconfig.json'),
  JSON.stringify({
    compilerOptions: { target: 'ES2022', module: 'ESNext', moduleResolution: 'bundler', jsx: 'react-jsx', strict: true, skipLibCheck: true, noEmit: true },
    include: ['src'],
  }),
)
execFileSync('pnpm', ['install', '--prefer-offline', '--ignore-workspace'], { cwd: dir, stdio: 'inherit' })

// A component is a function or a React exotic object (`forwardRef`, `memo`); a namespace object (Chakra's `Accordion`)
// holds components under its keys, which become `Accordion.Root`-style wrappers.
const isComponent = (value) => typeof value === 'function' || (typeof value === 'object' && value !== null && '$$typeof' in value)
const require = createRequire(join(dir, 'package.json'))
rmSync(join(dir, 'src'), { recursive: true, force: true })
mkdirSync(join(dir, 'src'))
let written = 0
for (const { name, prefix } of libraries) {
  const exports = await import(require.resolve(name))
  for (const [key, value] of Object.entries(exports)) {
    if (!/^[A-Z]/.test(key) || value === null || value === undefined) continue
    const paths = isComponent(value)
      ? [[key, key]]
      : typeof value === 'object'
        ? Object.entries(value).filter(([part, inner]) => /^[A-Z]/.test(part) && isComponent(inner)).map(([part]) => [`${key}${part}`, `${key}.${part}`])
        : []
    for (const [id, access] of paths) {
      const source = `import type { ComponentProps } from 'react'\nimport { ${key} } from '${name}'\nexport function ${prefix}${id}(props: ComponentProps<typeof ${access}>) {\n  return null\n}\n`
      writeFileSync(join(dir, 'src', `${prefix}${id}.tsx`), source)
      written += 1
    }
  }
}
console.log(`${written} wrapper components in ${join(dir, 'src')}`)
