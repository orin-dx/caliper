// Writes src/vocabulary.ts as src/vocabulary.json, for the readers that are not TypeScript.
import { writeFileSync } from 'node:fs'
import { vocabulary } from './vocabulary.ts'

writeFileSync(new URL('vocabulary.json', import.meta.url), `${JSON.stringify(vocabulary, null, 2)}\n`)
