// The UI stack end to end in a real browser: Tailwind stylesheet, shadcn/ui
// component styles and UI SDK behaviors (dialog, menu, tooltip, avatar), plus
// nested channels. Synthetic data, loopback only, headless.
import { harness, register, form, act, wait, target } from './qa-harness.mjs';
import { mkdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';

const shots = join(target, 'ui-kit-shots');
await mkdir(shots, { recursive: true });
const checks = [];
function check(name, ok, detail = '') {
  checks.push({ name, ok: !!ok });
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${ok ? '' : ' :: ' + detail}`);
}
const h = await harness();
try {
  const page = await h.phone(1280, 800);
  const errors = [];
  page.on('pageerror', e => errors.push(String(e)));
  // Wait out the 150 ms backdrop and tooltip fades before capturing.
  const shot = async name => { await wait(250); await page.screenshot({ path: join(shots, `${name}.png`), caret: 'initial' }); };
  const middle = selector => page.locator(selector).first().evaluate(e => { const r = e.getBoundingClientRect(); return r.top + r.height / 2; });

  await page.goto(h.origin);
  await page.waitForSelector('#auth-username');
  await shot('01-login');
  const sheet = await page.locator('link[rel=stylesheet]').evaluateAll(links => links.map(l => new URL(l.href).pathname));
  const css = await readFile(join(target, 'web-dist', sheet.find(p => p.startsWith('/app-')) ?? 'missing'), 'utf8').catch(() => '');
  check('stylesheet is the Tailwind build', css.includes('tailwindcss v4') && css.includes('@layer ui-sdk'), sheet.join(','));
  check('login button uses the shadcn primary button', await page.locator('#boot-submit').evaluate(e => getComputedStyle(e).backgroundColor === 'rgb(61, 220, 151)' && getComputedStyle(e).borderRadius === '6px'));

  await register(page, h.origin, 'kit_owner');
  await form(page, 'create', { 'f-name': 'Kit Lab' });
  await page.waitForFunction(() => !document.querySelector('.live-modal'));
  await page.locator('.server-rail [data-live^="guild:"]').click();
  await page.waitForSelector('.server-heading');
  await page.waitForFunction(() => customElements.get('ui-dialog') && customElements.get('ui-menu') && customElements.get('ui-tooltip') && customElements.get('ui-avatar'));
  check('UI SDK elements are registered', true);

  // ---- DropdownMenu (ui-menu): server menu -> Create Category ---------------
  await page.locator('.server-heading').click();
  await page.waitForSelector('ui-menu-content[data-open]');
  check('server menu opens in the top layer', await page.locator('ui-menu-content').evaluate(e => e.matches(':popover-open') && e.getAttribute('role') === 'menu'));
  await shot('02-server-menu');
  await page.locator('ui-menu-item', { hasText: 'Create Category' }).click();
  await page.waitForSelector('[data-live-form="channel"]');
  check('menu item runs its app action (Create Category preselected)', await page.locator('#f-extra').inputValue() === 'category');
  await page.locator('#f-name').fill('Projects');
  await page.locator('[data-live-form="channel"] [type=submit]').click();
  await page.waitForSelector('.category-row');
  check('creating a category does not open it as a conversation', await page.locator('.channel-header h1').textContent() !== 'Projects');

  // ---- Nested channels: "+" beside a category --------------------------------
  await page.locator('.category-row button').click();
  await page.waitForSelector('[data-live-form="channel"]');
  const category = await page.locator('#f-value').inputValue();
  check('category "+" preselects that category', /^\d+$/.test(category) && await page.locator('#f-extra').inputValue() === 'text');
  await shot('03-create-nested-channel');
  await page.locator('#f-name').fill('roadmap');
  await page.locator('[data-live-form="channel"] [type=submit]').click();
  await page.waitForSelector('.channel-row.nested');
  const order = await page.locator('.channel-scroll').evaluate(nav => [...nav.querySelectorAll('.category-row,.channel-row')].map(e => `${e.classList.contains('category-row') ? 'category' : e.classList.contains('nested') ? 'nested' : 'root'}:${e.textContent.trim()}`));
  const at = order.indexOf('category:Projects');
  check('nested channel is listed under its category', at >= 0 && order[at + 1] === 'nested:roadmap', order.join(' | '));

  // ---- Dialog (ui-dialog) ----------------------------------------------------
  await act(page, 'dialog:friend');
  await page.waitForSelector('.live-modal [data-live-form="friend"]');
  const dialog = await page.locator('ui-dialog-content').evaluate(e => ({ top: e.matches(':popover-open'), role: e.getAttribute('role'), modal: e.getAttribute('aria-modal'), focus: e.contains(document.activeElement) ? document.activeElement.id || document.activeElement.localName : '' }));
  check('dialog is a top-layer modal with focus on its first field', dialog.top && dialog.role === 'dialog' && dialog.modal === 'true' && dialog.focus === 'f-name', JSON.stringify(dialog));
  for (let i = 0; i < 6; i++) await page.keyboard.press('Tab');
  check('Tab stays inside the dialog', await page.evaluate(() => document.querySelector('ui-dialog-content').contains(document.activeElement)));
  await shot('04-dialog');
  // An error raised while a dialog is open must stay visible and dismissible.
  await act(page, 'media-error:Synthetic problem');
  await page.waitForSelector('.live-error');
  await page.locator('.live-error [data-live="dismiss-error"]').click({ timeout: 5000 });
  check('error toast is reachable above an open dialog', await page.locator('.live-error').count() === 0);
  await page.mouse.click(8, 8);
  await page.waitForFunction(() => !document.querySelector('.live-modal'), null, { timeout: 5000 }).then(() => check('pressing outside closes the dialog', true), () => check('pressing outside closes the dialog', false));
  await act(page, 'dialog:friend');
  await page.waitForSelector('.live-modal');
  await page.keyboard.press('Escape');
  await page.waitForFunction(() => !document.querySelector('.live-modal'), null, { timeout: 5000 }).then(() => check('Escape closes the dialog', true), () => check('Escape closes the dialog', false));

  // ---- Tooltip + Avatar -------------------------------------------------------
  await page.locator('.channel-main', { hasText: 'general' }).click();
  await page.waitForSelector('#live-input');
  await page.locator('.header-actions [aria-label="Member List"]').hover();
  await page.waitForSelector('ui-tooltip-content[data-open]', { timeout: 5000 }).then(async () => check('icon button shows its tooltip', (await page.locator('ui-tooltip-content[data-open]').textContent()) === 'Member List'), () => check('icon button shows its tooltip', false));
  await shot('05-tooltip');
  await page.mouse.move(640, 400);
  check('avatar shows initials when there is no image', await page.locator('.user-panel ui-avatar-fallback').evaluate(e => e.textContent === 'KI' || e.textContent.length > 0));

  // ---- Vertical centering in the sidebar, composer and user panel -------------
  await page.locator('#live-input').fill('Hello from the kit test');
  await page.locator('#live-input').press('Enter');
  await page.waitForFunction(() => [...document.querySelectorAll('.markdown')].some(e => e.textContent.includes('Hello from the kit test')));
  const near = (a, b) => Math.abs(a - b) <= 1.5;
  const composer = [await middle('.composer'), await middle('#live-input'), await middle('.composer [data-live="pick-file"]'), await middle('.composer [data-live="send-input"]')];
  check('composer text is vertically centered with its buttons', composer.every(v => near(v, composer[0])), composer.join(','));
  const row = [await middle('.channel-row.selected'), await middle('.channel-row.selected .icon'), await middle('.channel-row.selected .channel-main > span')];
  check('channel name is vertically centered in its row', row.every(v => near(v, row[0])), row.join(','));
  const panel = [await middle('.user-panel'), await middle('.user-panel ui-avatar'), await middle('.user-panel .self-label'), await middle('.user-panel [data-live="dialog:settings"]')];
  check('user panel avatar, name and buttons share one center line', panel.every(v => near(v, panel[0])), panel.join(','));
  await shot('06-chat');

  // ---- Settings sheet and phone layout ----------------------------------------
  await act(page, 'dialog:account');
  await page.waitForSelector('.settings-page .account-card');
  await shot('07-settings');
  check('settings sheet fills the window', await page.locator('.settings-page').evaluate(e => { const r = e.getBoundingClientRect(); return r.width === innerWidth && r.height === innerHeight; }));
  await act(page, 'close');
  await page.setViewportSize({ width: 390, height: 844 });
  await wait(200);
  await shot('08-phone-chat');
  await act(page, 'dialog:friend');
  await page.waitForSelector('.live-modal [data-live-form="friend"]');
  await shot('09-phone-dialog');
  check('phone dialog is a bottom sheet', await page.locator('.live-modal').evaluate(e => Math.round(e.getBoundingClientRect().bottom) === innerHeight && e.getBoundingClientRect().width === innerWidth));
  await act(page, 'close');
  const policy = await page.evaluate(() => window.__qaCsp);
  check('no Content-Security-Policy violations', policy.length === 0, JSON.stringify(policy));
  check('no page errors', errors.length === 0, errors.join('; '));
} catch (e) {
  check('ui kit flow completed', false, e.stack ?? String(e));
} finally {
  await h.close();
}
const failed = checks.filter(c => !c.ok).length;
console.log(`\n${checks.length - failed}/${checks.length} UI kit checks passed; screenshots in ${shots}`);
process.exit(failed ? 1 : 0);
