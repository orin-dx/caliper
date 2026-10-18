import { spawnSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

mkdirSync('./baselines', { recursive: true })

function runScript(script: string): string {
  const result = spawnSync('tsx', [resolve(import.meta.dirname, script)], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'], // stdout captured, stderr forwarded
    // Node's spawnSync default maxBuffer is 1MB — with 20 real fixture libraries
    // the RDT baseline alone now exceeds that, silently truncating stdout
    // mid-JSON-string (no non-zero exit, no thrown error) rather than failing
    // loudly. 100MB is generous headroom for however large fixtures/ grows.
    maxBuffer: 100 * 1024 * 1024,
  })
  if (result.error) throw new Error(`${script} failed to spawn: ${result.error.message}`)
  if (result.status !== 0) throw new Error(`${script} exited with status ${result.status}`)
  return result.stdout
}

console.log('Running react-docgen baseline...')
writeFileSync('./baselines/react-docgen.json', runScript('run-react-docgen.ts'))
console.log('✅ react-docgen baseline saved')

console.log('Running react-docgen-typescript baseline...')
writeFileSync('./baselines/react-docgen-typescript.json', runScript('run-react-docgen-typescript.ts'))
console.log('✅ react-docgen-typescript baseline saved')

console.log('Running semantic (tsc --api) baseline...')
writeFileSync('./baselines/semantic.json', runScript('run-semantic.ts'))
console.log('✅ semantic baseline saved')

console.log('Running caliper-docgen baseline (html-attributes=full, fair vs RDT)...')
writeFileSync('./baselines/caliper-docgen.json', runScript('run-ours.ts'))
console.log('✅ caliper-docgen baseline saved')

// react-docgen resolves zero `extends`/inherited-interface props of its own — comparing
// our html-attributes=full output (hundreds of synthesized inherited attrs per component)
// against it would manufacture a huge "extra in ours" count that's really just a
// no-inheritance-resolution-at-all vs some-resolution mismatch, not a real quality gap.
// A separate html-attributes=none baseline is the fair apples-to-apples comparator here.
console.log('Running caliper-docgen baseline (html-attributes=none, fair vs react-docgen)...')
process.env.HTML_ATTRIBUTES_MODE = 'none'
writeFileSync('./baselines/caliper-docgen-none.json', runScript('run-ours.ts'))
console.log('✅ caliper-docgen (none) baseline saved')

console.log('\nBaselines saved to ./baselines/')
