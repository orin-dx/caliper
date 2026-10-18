#!/usr/bin/env bash
# Installs the packed vite-plugin into an empty project and checks @orin-dx/caliper-docgen-napi arrives only as its
# transitive dependency. Usage: scripts/verify-install.sh <npm|pnpm>
set -euo pipefail

pm=${1:?usage: verify-install.sh <npm|pnpm>}
cd "$(dirname "$0")/.."

packed=$(mktemp -d)
project=$(mktemp -d)
trap 'rm -rf "$packed" "$project"' EXIT

pnpm --filter @orin-dx/caliper-docgen-vite-plugin build
pnpm --filter @orin-dx/caliper-docgen-napi --filter @orin-dx/caliper-docgen-vite-plugin pack --pack-destination "$packed"

# proto resolves pnpm from .prototools, which the temp project sits outside of.
PROTO_PNPM_VERSION=$(pnpm --version)
export PROTO_PNPM_VERSION

napi=$(ls "$packed"/caliper-docgen-napi-*.tgz)
plugin=$(ls "$packed"/caliper-docgen-vite-plugin-*.tgz)

# The override serves napi from this checkout's tarball instead of the registry. pnpm 11 reads overrides from
# pnpm-workspace.yaml, not package.json.
cd "$project"
if [ "$pm" = npm ]; then
  printf '{"private":true,"overrides":{"@orin-dx/caliper-docgen-napi":"file:%s"}}\n' "$napi" > package.json
  npm install "$plugin"
else
  echo '{"private":true}' > package.json
  printf 'overrides:\n  "@orin-dx/caliper-docgen-napi": "file:%s"\n' "$napi" > pnpm-workspace.yaml
  pnpm add "$plugin"
fi

# Resolve napi from vite-plugin's own location: pnpm doesn't hoist transitive deps into node_modules/, and the
# plugin's `exports` has only an `import` condition, so `require.resolve` can't reach it from here.
node --input-type=module <<'EOF'
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';

const { dependencies = {} } = JSON.parse(readFileSync('package.json', 'utf8'));
if (dependencies['@orin-dx/caliper-docgen-napi']) {
  throw new Error('napi is a direct dependency; the check proves nothing');
}
const plugin = import.meta.resolve('@orin-dx/caliper-docgen-vite-plugin');
createRequire(plugin).resolve('@orin-dx/caliper-docgen-napi/package.json');
console.log('OK');
EOF
