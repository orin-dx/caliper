#!/usr/bin/env node
// Checks the extractor's interface collection against TypeScript's own checker.
//
// The rule under test: a same-name interface declaration belongs to the lexical scope that declares it. Declarations
// in one scope merge (declaration merging); declarations in different scopes are different types. What the extractor
// collects must not depend on declaration order or on how many other scopes declare the same name.
//
// Every sequence of one to MAX_DECLS declarations over four scopes (module, function body, function nested in that
// function, bare block) is turned into a .tsx file. TypeScript says which property sets exist (one per declaring
// scope); the extractor, run by the ignored Rust test `interface_scopes_match_typescript`, must collect the same
// multiset of property sets. Usage: node scripts/interface_scope_oracle.mjs [max-declarations]
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(join(root, 'apps/oracle/package.json'));
const ts = require('typescript');

const SCOPES = ['module', 'function', 'nested', 'block'];
const MAX_DECLS = Number(process.argv[2] ?? 3);

function* sequences(length) {
  if (length === 0) {
    yield [];
    return;
  }
  for (const rest of sequences(length - 1)) for (const scope of SCOPES) yield [...rest, scope];
}

/** Renders one sequence; declaration `i` declares property `p<i>`. Containers open where their scope first appears. */
function render(sequence) {
  const decl = (i) => `interface Props { p${i}: string }`;
  const mod = [];
  const fn = [];
  const nested = [];
  const block = [];
  let fnAt = -1;
  let blockAt = -1;
  sequence.forEach((scope, i) => {
    if (scope === 'module') mod.push([i, decl(i)]);
    if (scope === 'function' || scope === 'nested') {
      if (fnAt < 0) fnAt = i;
    }
    if (scope === 'function') fn.push(decl(i));
    if (scope === 'nested') nested.push(decl(i));
    if (scope === 'block') {
      if (blockAt < 0) blockAt = i;
      block.push(decl(i));
    }
  });
  const items = mod.map(([i, text]) => [i, text]);
  if (fnAt >= 0) {
    const inner = nested.length ? [`function g() { ${nested.join(' ')} }`] : [];
    // Nested declarations that come before any function-scope one still sit inside the function body.
    items.push([fnAt, `function f() { ${[...fn, ...inner].join(' ')} }`]);
  }
  if (blockAt >= 0) items.push([blockAt, `{ ${block.join(' ')} }`]);
  items.sort((a, b) => a[0] - b[0]);
  return `${items.map(([, text]) => text).join('\n')}\nexport function Box(props: Props) { return <div />; }\n`;
}

function expected(source) {
  const file = 'case.tsx';
  const host = ts.createCompilerHost({});
  const original = host.getSourceFile.bind(host);
  host.getSourceFile = (name, ...rest) =>
    name === file ? ts.createSourceFile(name, source, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TSX) : original(name, ...rest);
  host.fileExists = (name) => name === file;
  const program = ts.createProgram([file], { jsx: ts.JsxEmit.Preserve, noLib: true, types: [] }, host);
  const checker = program.getTypeChecker();
  const sets = new Map();
  const visit = (node) => {
    if (ts.isInterfaceDeclaration(node)) {
      const symbol = checker.getSymbolAtLocation(node.name);
      if (symbol && !sets.has(symbol)) {
        const type = checker.getDeclaredTypeOfSymbol(symbol);
        sets.set(symbol, checker.getPropertiesOfType(type).map((p) => p.name).sort());
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(program.getSourceFile(file));
  return [...sets.values()].map((names) => names.join(',')).sort();
}

const cases = [];
for (let length = 1; length <= MAX_DECLS; length += 1) for (const sequence of sequences(length)) cases.push(sequence);
const sources = cases.map(render);

const dir = mkdtempSync(join(tmpdir(), 'interface-scope-'));
const input = join(dir, 'cases.json');
const output = join(dir, 'out.json');
writeFileSync(input, JSON.stringify(sources));
execFileSync(
  'cargo',
  ['test', '-p', 'caliper-docgen-core', '--lib', 'interface_scopes_match_typescript', '--', '--ignored', '--exact', 'extractor::tests::interface_scopes_match_typescript'],
  { cwd: root, stdio: ['ignore', 'ignore', 'inherit'], env: { ...process.env, INTERFACE_SCOPE_CASES: input, INTERFACE_SCOPE_OUT: output } },
);
const actual = JSON.parse(readFileSync(output, 'utf8')).map((sets) => sets.slice().sort());

let mismatches = 0;
const classes = new Map();
cases.forEach((sequence, index) => {
  const want = expected(sources[index]);
  const got = actual[index];
  if (JSON.stringify(want) !== JSON.stringify(got)) {
    mismatches += 1;
    const key = `${sequence.join(' > ')}`;
    classes.set(key, { want, got });
  }
});
for (const [key, { want, got }] of classes) console.log(`MISMATCH ${key}\n  typescript: ${JSON.stringify(want)}\n  extractor:  ${JSON.stringify(got)}`);
console.log(`${cases.length} cases, ${mismatches} mismatches`);
process.exit(mismatches === 0 ? 0 : 1);
