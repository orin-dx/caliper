// The path of the pinned tsrs native binary. The `tsrs` npm package (bin/tsrs) is a Node launcher; the binary itself ships
// in a per-platform optional dependency (`@tsrs/darwin-arm64`, `@tsrs/linux-x64-gnu`, `@tsrs/linux-x64-musl`, ...). The
// package's own resolver (lib/getExePath.js) picks that package for this machine, including the glibc/musl choice on
// Linux, so this asks it rather than guessing a path. `TSRS_BIN` wins when set. Prints the path when run directly:
// `just test-checker-lane` uses that as TSRS_BIN for the checker lane.
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { pathToFileURL } from 'node:url'

/** The tsrs binary for this machine, or an error naming what is missing. */
export async function resolveTsrsBin(): Promise<string> {
  const override = process.env.TSRS_BIN
  if (override) return override
  const pkg = createRequire(import.meta.url).resolve('tsrs/package.json')
  const resolver = await import(pathToFileURL(join(dirname(pkg), 'lib', 'getExePath.js')).href)
  return resolver.default() as string
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  try {
    console.log(await resolveTsrsBin())
  } catch (error) {
    console.error(`tsrs-bin: ${error instanceof Error ? error.message : String(error)}`)
    process.exit(1)
  }
}
