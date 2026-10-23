// Run before release compilation and from Node tests. Inspect Cargo's actual
// wasm normal/build graph, not source strings or the native development graph.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = fileURLToPath(new URL('../', import.meta.url));
const FORBIDDEN = /\b(?:m68000-emu|tje-host|tje-engine-verify|tje-engine-cranelift) v[0-9]/;

export function checkDependencies() {
  const metadata = spawnSync('cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1', '--manifest-path', `${ROOT}/Cargo.toml`], { encoding: 'utf8' });
  assert.equal(metadata.status, 0, metadata.stderr);
  const frontend = JSON.parse(metadata.stdout).packages.find(package_ => package_.name === 'tje-web');
  assert.deepEqual(frontend.features.default, [], 'shipping must not enable replay observers by default');
  const results = [];
  for (const observer of [false, true]) {
    const args = ['tree', '--locked', '--manifest-path', `${ROOT}/Cargo.toml`, '-p', 'tje-web', '--target', 'wasm32-unknown-unknown', '--edges', 'normal,build', '--format', '{p} [{f}]'];
    if (observer) args.push('--features', 'replay-observer');
    const tree = spawnSync('cargo', args, { encoding: 'utf8' });
    assert.equal(tree.status, 0, tree.stderr);
    assert(!FORBIDDEN.test(tree.stdout), `interpreter/oracle/native backend leaked into ${observer ? 'observer' : 'shipping'} wasm graph:\n${tree.stdout}`);
    assert(!/\bgenesis-core v[^\n]*\[[^\]]*\bemulator\b/.test(tree.stdout), 'vendored peripheral types must not enable their 68000 emulator feature');
    assert(!/browser-proof|reference-export/.test(tree.stdout), 'browser oracle features must never be enabled');
    if (!observer) assert(!/replay-observer/.test(tree.stdout), 'release graph must not contain replay observers');
    assert(tree.stdout.includes('tje-engine-wasmgen v'), 'the real compiled engine must be linked');
    results.push({ graph: observer ? 'observation-only' : 'shipping', interpreterFree: true });
  }
  const script = readFileSync(new URL('./build.sh', import.meta.url), 'utf8');
  assert(script.includes('--no-default-features'), 'release build must explicitly exclude optional observer features');
  assert(!/--all-features|--features[ =].*(?:replay-observer|browser-proof|reference-export)/.test(script), 'shipping build cannot enable development features');
  return results;
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  console.log(JSON.stringify(checkDependencies()));
}
