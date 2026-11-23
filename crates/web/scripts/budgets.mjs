// Size budgets for the packaged web app (docs/features/perf.md).
//
// Every WASM module and the first screen's JavaScript have a Brotli budget
// in budgets.json: the measured size plus a small margin. A feature that adds
// to the core, or grows a module past its budget, fails here and in the audit
// (`npm run audit`) instead of slowly bloating the first load.
//
// Run: node scripts/budgets.mjs [dist]   (default $CARGO_TARGET_DIR/web-dist)
import { readFile } from 'node:fs/promises';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { brotliCompressSync, constants } from 'node:zlib';

const here = dirname(fileURLToPath(import.meta.url));
export async function checkBudgets(dist) {
  const budgets = JSON.parse(await readFile(join(here, 'budgets.json'), 'utf8'));
  const inventory = JSON.parse(await readFile(join(dist, 'sizes.json'), 'utf8'));
  const failures = [], report = [];
  const over = (name, size, budget) => {
    report.push(`${name}: ${size} B (budget ${budget} B)`);
    if (size > budget) failures.push(`${name} is ${size} B Brotli, over its ${budget} B budget`);
  };
  // Reserved headroom (task `headroom`, docs/features/perf.md): each allowance
  // is bytes set aside for one planned change. A budget plus every allowance
  // must stay within its ceiling, so raising a budget means taking from an
  // allowance on purpose (and saying so in budgets.json), never silently.
  for (const key of ['core', 'firstLoad', 'firstScreenJs']) {
    const reserved = Object.values(budgets.allowances ?? {}).reduce((n, a) => n + (a[key] ?? 0), 0);
    const ceiling = budgets.ceilings?.[key];
    if (ceiling === undefined) continue;
    report.push(`${key}: budget ${budgets[key]} B + allowances ${reserved} B (ceiling ${ceiling} B)`);
    if (budgets[key] + reserved > ceiling)
      failures.push(`${key} budget ${budgets[key]} B plus allowances ${reserved} B is over its ${ceiling} B ceiling: take the bytes from an allowance in scripts/budgets.json`);
  }
  const wasm = inventory.assets.filter((a) => /^pkg\/.*\.wasm$/.test(a.file));
  const core = wasm.find((a) => a.file.endsWith('_bg.wasm'));
  over('core WASM', core.brotli, budgets.core);
  over('first load (installable shell inventory)', inventory.firstLoadBytes, budgets.firstLoad);
  // Split modules are named after their split point: `split_<name>[_<hash>].wasm`.
  // Shared chunks (`chunk_<n>.wasm`) are numbered by the splitter, so only the
  // largest one and their total have budgets.
  const named = new Map();
  let shared = 0, largestShared = 0;
  for (const a of wasm) {
    if (a === core) continue;
    const m = a.file.match(/\/split_(.+?)(?:_\d{6,})?\.wasm$/);
    if (m) named.set(m[1], (named.get(m[1]) ?? 0) + a.brotli);
    else {
      shared += a.brotli;
      largestShared = Math.max(largestShared, a.brotli);
    }
  }
  for (const [name, size] of [...named].sort()) {
    const budget = budgets.modules[name];
    if (budget === undefined) failures.push(`split module "${name}" (${size} B) has no budget in scripts/budgets.json`);
    else over(`module ${name}`, size, budget);
  }
  for (const name of Object.keys(budgets.modules))
    if (!named.has(name)) failures.push(`budgets.json lists module "${name}", which the build no longer has`);
  over('largest shared chunk', largestShared, budgets.largestSharedChunk);
  over('all shared chunks', shared, budgets.sharedChunks);
  // The first screen's JavaScript: what live.js imports statically, and the glue.
  const seen = new Set();
  let js = 0;
  const visit = async (file) => {
    if (seen.has(file)) return;
    seen.add(file);
    const text = await readFile(join(dist, file), 'utf8');
    js += brotliCompressSync(Buffer.from(text), { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length;
    for (const [, spec] of text.matchAll(/(?:\bfrom|\bimport)\s*["']([^"']+\.js)["']/g))
      await visit(spec.startsWith('/') ? spec.slice(1) : join(dirname(file), spec).replaceAll('\\', '/'));
  };
  const live = inventory.assets.find((a) => /^public\/runtime\/[a-f0-9]{20}\/live\.js$/.test(a.file));
  await visit(live.file);
  await visit(core.file.replace(/_bg\.wasm$/, '.js'));
  over('first-screen JavaScript', js, budgets.firstScreenJs);
  return { failures, report };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const dist = process.argv[2] ?? process.env.STICKTALK_WEB_DIST ?? join(process.env.CARGO_TARGET_DIR ?? join(here, '../../../target'), 'web-dist');
  const { failures, report } = await checkBudgets(dist);
  console.log(report.join('\n'));
  if (failures.length) {
    console.error('\nSize budgets exceeded:\n- ' + failures.join('\n- '));
    process.exit(1);
  }
  console.log('Size budgets: passed');
}
