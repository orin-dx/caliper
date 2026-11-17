// Where a declaration lives, as the oracle and the weekly read it: in a dependency, in `@types/react`, or in one of
// TypeScript's lib files. The patterns are in src/path-classes.json, which scripts/oracle_data.py reads too, and both
// sides run the cases of tests/fixtures/path-class-cases.json (tests/path-class.test.ts, scripts/test_path_classes.py),
// so a path cannot be a dependency to one and a project file to the other. A path is read with `/` separators, so a
// Windows path (`C:\app\node_modules\x`) is classed as its POSIX spelling is.
import { readFileSync } from 'node:fs'

interface Rules {
  dependency: string
  reactTypes: string
  libFile: string
}

const rules: Rules = JSON.parse(readFileSync(new URL('path-classes.json', import.meta.url), 'utf8'))
const DEPENDENCY = new RegExp(rules.dependency)
const REACT_TYPES = new RegExp(rules.reactTypes)
const LIB_FILE = new RegExp(rules.libFile)

const slashed = (path: string): string => path.replaceAll('\\', '/')

/** Whether the file is installed under a `node_modules` directory, `@types/react` and TypeScript's own lib files
 * included. */
export const isDependency = (path: string): boolean => DEPENDENCY.test(slashed(path))

/** Whether the file is part of `@types/react`: React's own declarations, which Caliper's output leaves out as inherited. */
export const isReactTypes = (path: string): boolean => REACT_TYPES.test(slashed(path))

/** Whether the file is one of TypeScript's `lib.*.d.ts`: the members of `String`, `Function` and the other globals. */
export const isLibFile = (path: string): boolean => LIB_FILE.test(slashed(path))
