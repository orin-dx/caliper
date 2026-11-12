#!/usr/bin/env bash
# Installs the packed CLI and napi packages, laid out as a release publishes them (each binary in its platform package,
# which depends on the pinned @tsrs/<platform>), into an empty project with no TypeScript and no TSRS_BIN, and checks
# that both find the tsrs installed beside them and that it answers for one component.
# Usage: scripts/verify-checker-install.sh <npm|pnpm> <caliper-docgen binary> <napi .node addon>
set -euo pipefail

pm=${1:?usage: verify-checker-install.sh <npm|pnpm> <caliper-docgen binary> <napi addon>}
cli_bin=$(cd "$(dirname "${2:?missing the caliper-docgen binary}")" && pwd)/$(basename "$2")
addon=$(cd "$(dirname "${3:?missing the napi addon}")" && pwd)/$(basename "$3")
cd "$(dirname "$0")/.."
repo=$PWD

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/packed" "$work/project"

platform=$(node -p "
  const { platform, arch } = process
  const libc = platform === 'linux' ? (process.report.getReport().header.glibcVersionRuntime ? '-gnu' : '-musl') : ''
  platform === 'win32' ? \`win32-\${arch}-msvc\` : \`\${platform}-\${arch}\${libc}\`
")
[ -d "packages/cli/npm/$platform" ] || { echo "no caliper-docgen package for $platform" >&2; exit 1; }
unset TSRS_BIN

# Copies, so nothing lands in the checkout: the binaries go into their platform packages as the release places them,
# and the napi wrapper is packed without a locally built addon, which its loader would otherwise prefer.
cp -R "packages/cli/npm/$platform" "$work/cli-platform"
cp "$cli_bin" "$work/cli-platform/caliper-docgen"
chmod +x "$work/cli-platform/caliper-docgen"
cp -R "packages/napi/npm/$platform" "$work/napi-platform"
cp "$addon" "$work/napi-platform/caliper_docgen_napi.$platform.node"
mkdir "$work/napi"
cp packages/napi/package.json packages/napi/index.js packages/napi/index.d.ts packages/napi/binding.d.ts "$work/napi/"
for dir in "$work/cli-platform" "$work/napi-platform" "$work/napi" "$repo/packages/cli"; do
  (cd "$dir" && npm pack --silent --pack-destination "$work/packed" > /dev/null)
done
ls -l "$work/packed"

cli=$(ls "$work"/packed/caliper-docgen-[0-9]*.tgz)
napi=$(ls "$work"/packed/caliper-docgen-napi-[0-9]*.tgz)
cli_platform=$(ls "$work"/packed/caliper-docgen-"$platform"-*.tgz)
napi_platform=$(ls "$work"/packed/caliper-docgen-napi-"$platform"-*.tgz)

# proto resolves pnpm from .prototools, which the temp project sits outside of.
PROTO_PNPM_VERSION=$(pnpm --version)
export PROTO_PNPM_VERSION

cd "$work/project"
mkdir src
printf '%s\n' \
  "export interface ButtonProps { label: string; size?: 'sm' | 'lg' }" \
  'export function Button(props: ButtonProps) { return <button>{props.label}</button> }' > src/Button.tsx
if [ "$pm" = npm ]; then
  printf '{"private":true,"overrides":{"caliper-docgen-%s":"file:%s","caliper-docgen-napi-%s":"file:%s"}}\n' \
    "$platform" "$cli_platform" "$platform" "$napi_platform" > package.json
  npm install --no-audit --no-fund "$cli" "$napi"
else
  echo '{"private":true}' > package.json
  printf 'overrides:\n  "caliper-docgen-%s": "file:%s"\n  "caliper-docgen-napi-%s": "file:%s"\n' \
    "$platform" "$cli_platform" "$platform" "$napi_platform" > pnpm-workspace.yaml
  pnpm add "$cli" "$napi"
fi

./node_modules/.bin/caliper-docgen extract --src src --experimental-semantic-types --json > cli.json
node --input-type=module - "$PWD" <<'EOF'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { join } from 'node:path'

const project = process.argv[2]
const expected = '1 component(s) resolved by the TypeScript checker'
const checker = (diagnostics) => diagnostics.filter((d) => d.code === 'SEMANTIC_BACKEND').map((d) => d.message)

const cli = checker(JSON.parse(readFileSync(join(project, 'cli.json'), 'utf8')).diagnostics)
if (JSON.stringify(cli) !== JSON.stringify([expected])) throw new Error(`CLI: ${JSON.stringify(cli)}`)
console.log(`CLI: ${cli[0]}`)

const napi = createRequire(join(project, 'package.json'))('caliper-docgen-napi')
const options = {
  srcDirs: [join(project, 'src')],
  lazy: true,
  semanticTypes: 'on-demand',
  semanticWaitMs: 60_000,
  semanticWaitEmptyMs: 60_000,
}
const session = napi.createSession(options)
const index = JSON.parse(await napi.initializeSessionIndex(session, options))
const answers = JSON.parse(napi.sessionComponentAnswers(session, Object.keys(index.components)))
napi.closeSession(session)
const addon = Object.values(answers).flatMap((answer) => checker(answer.diagnostics))
if (JSON.stringify(addon) !== JSON.stringify([expected])) throw new Error(`napi: ${JSON.stringify(addon)}`)
console.log(`napi: ${addon[0]}`)
EOF
