// Stylesheet and UI behavior assets for the packager.
// - Tailwind CSS compiles styles/app.css (theme + shadcn/ui component classes)
//   against the class names used in src/live, index.html and public/*.js.
// - UI SDK ships browser-ready ES modules; only the components public/ui.js
//   imports, and the chunks they reach, are copied next to it.
import {execFileSync} from 'node:child_process';
import {readFile,writeFile,mkdir,copyFile} from 'node:fs/promises';
import {dirname,join,resolve} from 'node:path';

const modules=resolve('node_modules');
const missing=name=>new Error(`${name} is not installed. Run "npm ci --ignore-scripts" in crates/web first.`);

/** Compiles a stylesheet into the task target and returns its bytes. */
export async function buildStyles(target, entry = 'app') {
  const cli=join(modules,'@tailwindcss/cli/dist/index.mjs');
  await readFile(cli).catch(()=>{throw missing('Tailwind CSS');});
  const out=join(target,'web-css',`${entry}.css`);
  await mkdir(dirname(out),{recursive:true});
  execFileSync(process.execPath,[cli,'-i',`styles/${entry}.css`,'-o',out,'--minify'],{stdio:['ignore','ignore','inherit'],windowsHide:true});
  return readFile(out);
}
/** Feature stylesheets loaded by their feature's script on first use, not
 * with the app: `styles/<name>.css` (it `@reference`s app.css) becomes
 * `public/<name>.css`. Served `no-cache` (unhashed), so a deploy replaces it. */
export const LAZY_STYLES=['activities','e2ee'];

/** Copies the UI SDK modules that public/ui.js imports into <public>/ui-sdk/<version>/. */
export async function copyUiSdk(publicDir) {
  const pkg=join(modules,'@ui-sdk/web');
  const {version}=JSON.parse(await readFile(join(pkg,'package.json'),'utf8').catch(()=>{throw missing('UI SDK');}));
  const ui=join(publicDir,'ui.js');
  let source=await readFile(ui,'utf8');
  const entries=[...source.matchAll(/'\/public\/ui-sdk\/(web\/[\w-]+\.js)'/g)].map(m=>m[1]);
  if(!entries.length)throw new Error('public/ui.js imports no UI SDK components');
  // The directory is versioned, so a cached module never outlives its release.
  const base=`ui-sdk/${version}`;
  const seen=new Set(), queue=[...entries];
  while(queue.length) {
    const file=queue.pop();
    if(seen.has(file))continue;
    seen.add(file);
    const text=await readFile(join(pkg,'browser',file),'utf8');
    const to=join(publicDir,base,file);
    await mkdir(dirname(to),{recursive:true});
    await writeFile(to,text);
    // Minified ES modules: every dependency is a relative ".js" string literal.
    for(const [,specifier] of text.matchAll(/["'`](\.\.?\/[^"'`\s]+\.js)["'`]/g))
      queue.push(join(dirname(file),specifier).replaceAll('\\','/'));
  }
  await copyFile(join(pkg,'LICENSE'),join(publicDir,base,'LICENSE'));
  await writeFile(ui,source.replaceAll('/public/ui-sdk/',`/public/${base}/`));
  return {version,files:seen.size};
}
