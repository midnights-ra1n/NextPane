// NextPane web UI. No build step, no dependencies. DOM is built with h(), never innerHTML.

const $app = document.getElementById('app');
let timers = [];
let me = null;

// ---------- helpers ----------

function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'value') el.value = v;
    else if (k === 'checked') el.checked = !!v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const c of kids.flat(Infinity)) if (c != null && c !== false) el.append(c instanceof Node ? c : String(c));
  return el;
}

function svg(tag, attrs, ...kids) {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [k, v] of Object.entries(attrs || {})) el.setAttribute(k, v);
  el.append(...kids);
  return el;
}

const ICONS = {
  overview: 'M3 3h7v7H3zM14 3h7v7h-7zM14 14h7v7h-7zM3 14h7v7H3z',
  hosts: 'M3 4h18v6H3zM3 14h18v6H3zM7 7h.01M7 17h.01',
  apps: 'M21 8l-9-5-9 5 9 5 9-5zM3 8v8l9 5 9-5V8M12 13v8',
  deployments: 'M12 8a4 4 0 100 8 4 4 0 000-8zM2 12h6M16 12h6',
  activity: 'M22 12h-4l-3 9L9 3l-3 9H2',
  settings: 'M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6',
  search: 'M11 4a7 7 0 100 14 7 7 0 000-14zM21 21l-4.3-4.3',
  plus: 'M12 5v14M5 12h14',
  logout: 'M9 21H5a2 2 0 01-2-2V5a2 2 0 012-2h4M16 17l5-5-5-5M21 12H9',
  menu: 'M3 6h18M3 12h18M3 18h18',
  alert: 'M12 9v4M12 17h.01M10.3 3.9L1.8 18a2 2 0 001.7 3h17a2 2 0 001.7-3L13.7 3.9a2 2 0 00-3.4 0z',
  check: 'M20 6L9 17l-5-5',
  arrow: 'M5 12h14M12 5l7 7-7 7',
};
const icon = (name, size = 16) => svg('svg', { viewBox: '0 0 24 24', width: size, height: size, fill: 'none', stroke: 'currentColor', 'stroke-width': '1.75', 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'aria-hidden': 'true', class: 'ic' }, svg('path', { d: ICONS[name] }));

async function api(path, { method = 'GET', body } = {}) {
  const r = await fetch(path, {
    method,
    headers: body !== undefined ? { 'content-type': 'application/json' } : {},
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const data = await r.json().catch(() => ({}));
  if (r.status === 401 && !path.startsWith('/api/login') && !path.startsWith('/api/setup')) {
    location.hash = '#/login';
    const e = new Error('signed out'); e.auth = true; throw e;
  }
  if (!r.ok) throw new Error(data.error || `${r.status} ${r.statusText}`);
  return data;
}

function toast(msg, err = false) {
  const t = h('div', { class: 'toast' + (err ? ' err' : ''), role: err ? 'alert' : 'status' }, icon(err ? 'alert' : 'check'), h('span', {}, msg));
  document.getElementById('toasts').append(t);
  setTimeout(() => t.remove(), err ? 8000 : 4000);
}

/** Run an async action from a button: disables it, toasts the result. */
async function act(btn, fn, okMsg) {
  const busy = (on) => { if (btn) { btn.disabled = on; btn.classList.toggle('busy', on); btn.toggleAttribute('aria-busy', on); } };
  busy(true);
  try {
    const r = await fn();
    if (okMsg) toast(typeof okMsg === 'function' ? okMsg(r) : okMsg);
    return r;
  } catch (e) {
    if (!e.auth) toast(e.message, true);
  } finally {
    busy(false);
  }
}

const KB = 1024;
function bytes(n) {
  if (!n) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(u.length - 1, Math.floor(Math.log(n) / Math.log(KB)));
  return `${(n / KB ** i).toFixed(i ? 1 : 0)} ${u[i]}`;
}
const pct = (n) => `${(n || 0).toFixed(1)}%`;
function ago(ts) {
  if (!ts) return 'never';
  const s = Math.max(0, Date.now() / 1000 - ts);
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}
function dur(a, b) {
  if (!a || !b) return '';
  const s = b - a;
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}
function uptime(s) {
  const d = Math.floor(s / 86400), hh = Math.floor((s % 86400) / 3600);
  return d ? `${d}d ${hh}h` : `${hh}h ${Math.floor((s % 3600) / 60)}m`;
}
const short = (sha) => (sha ? sha.slice(0, 7) : '—');
const wsOrigin = () => location.origin.replace(/^http/, 'ws');
const sum = (list, f) => list.reduce((s, x) => s + (f(x) || 0), 0);

function every(ms, fn) {
  timers.push(setInterval(fn, ms));
}

function copyBtn(text) {
  const b = h('button', { class: 'btn ghost sm copy', type: 'button', onclick: async () => {
    await navigator.clipboard.writeText(typeof text === 'function' ? text() : text);
    b.textContent = 'Copied';
    setTimeout(() => (b.textContent = 'Copy'), 1500);
  } }, 'Copy');
  return b;
}
const appLink = (ip, port) => {
  const url = `http://${ip.includes(':') ? `[${ip}]` : ip}:${port}`;
  return h('a', { class: 'mono', href: url, target: '_blank', rel: 'noopener' }, url);
};
const code = (text) => h('div', { class: 'code' }, text, copyBtn(text));

function field(label, input, hint) {
  return h('label', { class: 'field' }, h('span', {}, label), input, hint && h('small', {}, hint));
}

function bar(parts) {
  return h('div', { class: 'bar' }, parts.map(([cls, v]) => {
    const i = h('i', { class: cls });
    i.style.width = `${Math.max(0, Math.min(100, v || 0))}%`;
    return i;
  }));
}

/** Sparkline of one or two series (values 0..max). */
function spark(values, max, second) {
  const n = values.length;
  const H = 36;
  const wrap = svg('svg', { class: 'spark', viewBox: `0 0 100 ${H}`, preserveAspectRatio: 'none', 'aria-hidden': 'true' });
  if (n < 2) return wrap;
  const pts = (vs) => vs.map((v, i) => `${(i / (n - 1)) * 100},${H - 2 - (Math.min(v, max) / (max || 1)) * (H - 4)}`);
  const p = pts(values);
  wrap.append(svg('path', { class: 'a', d: `M0,${H}L${p.join('L')}L100,${H}Z` }), svg('path', { class: 'l', d: `M${p.join('L')}` }));
  if (second) wrap.append(svg('path', { class: 'l2', d: `M${pts(second).join('L')}` }));
  return wrap;
}

function modal(title, body, actions, cls = '') {
  // Enter in a field must not close the dialog: only explicit submit buttons (confirm) do.
  const d = h('dialog', { class: cls }, h('form', { method: 'dialog', onsubmit: (e) => !e.submitter && e.preventDefault() },
    h('div', { class: 'd-body' }, title && h('h2', {}, title), body),
    actions !== null && h('div', { class: 'd-foot' }, h('button', { class: 'btn', type: 'button', onclick: () => d.close('cancel') }, 'Close'), actions || [])));
  d.addEventListener('close', () => d.remove());
  document.body.append(d);
  d.showModal();
  return d;
}

function confirmDialog(title, text, label = 'Confirm', danger = true) {
  return new Promise((resolve) => {
    const d = modal(title, h('p', { class: 'muted pre' }, text), h('button', { class: 'btn ' + (danger ? 'danger' : 'primary'), value: 'ok' }, label));
    d.addEventListener('close', () => resolve(d.returnValue === 'ok'));
  });
}

function download(filename, content) {
  const a = h('a', { href: URL.createObjectURL(new Blob([content], { type: 'application/octet-stream' })), download: filename });
  document.body.append(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

const statusDotCls = (s) => ({ success: 'ok', failed: 'err', running: 'run' }[s] || '');

function select(options, value, label) {
  return h('select', { 'aria-label': label, class: 'auto' }, options.map(([v, l]) => h('option', { value: v, selected: v === value }, l)));
}

// ---------- layout ----------

const NAV = [
  ['#/', 'Overview', 'overview'],
  ['#/hosts', 'Hosts', 'hosts'],
  ['#/apps', 'Apps', 'apps'],
  ['#/deployments', 'Deployments', 'deployments'],
  ['#/activity', 'Activity', 'activity'],
  ['#/settings', 'Settings', 'settings'],
];

let sideTimer;
function ensureLayout() {
  if (document.getElementById('side')) return;
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform);
  const side = h('aside', { id: 'side', class: 'side' },
    h('div', { class: 'side-top' }, h('a', { href: '#/', class: 'brand' }, h('img', { src: '/logo.svg', alt: '' }), h('span', {}, 'NextPane')), h('span', { id: 'side-version', class: 'dim mono small' })),
    h('button', { class: 'search', type: 'button', onclick: palette }, h('span', { class: 'row' }, icon('search', 14), 'Search…'), h('kbd', {}, isMac ? '⌘K' : 'Ctrl K')),
    h('nav', { id: 'side-nav' }, NAV.map(([href, label, ic]) => h('a', { href, 'data-nav': href }, icon(ic), h('span', { class: 'grow' }, label), h('span', { class: 'count', id: `count-${ic}` })))),
    h('div', { class: 'group' }, h('span', {}, 'Hosts'), h('button', { class: 'icon-btn', title: 'Add host', 'aria-label': 'Add host', onclick: addHostDialog }, icon('plus', 14))),
    h('div', { id: 'side-hosts', class: 'side-hosts' }),
    h('div', { class: 'side-foot' }, h('span', { class: 'avatar' }, (me?.user || '?')[0].toUpperCase()), h('div', { class: 'grow' }, h('div', { class: 'small' }, me?.user || ''), h('div', { class: 'dim small' }, 'Administrator')),
      h('button', { class: 'icon-btn', title: 'Sign out', 'aria-label': 'Sign out', onclick: logout }, icon('logout'))));
  const mobile = h('div', { class: 'mobile-bar' }, h('button', { class: 'icon-btn', 'aria-label': 'Menu', onclick: () => document.body.classList.toggle('side-open') }, icon('menu', 18)),
    h('a', { href: '#/', class: 'brand' }, h('img', { src: '/logo.svg', alt: '' }), h('span', {}, 'NextPane')), h('button', { class: 'icon-btn', 'aria-label': 'Search', onclick: palette }, icon('search', 18)));
  $app.replaceChildren(h('div', { class: 'layout' }, side, h('div', { class: 'scrim', onclick: () => document.body.classList.remove('side-open') }), h('div', { class: 'main-col' }, mobile, h('main', { id: 'main' }))));
  api('/api/system').then((s) => { document.getElementById('side-version').textContent = `v${s.version}`; }).catch(() => {});
  refreshSide();
  clearInterval(sideTimer);
  sideTimer = setInterval(refreshSide, 10000);
}

async function refreshSide() {
  try {
    const [{ hosts, latest_agent }, { apps }] = await Promise.all([api('/api/hosts'), api('/api/apps')]);
    const list = document.getElementById('side-hosts');
    if (!list) return;
    list.replaceChildren(...(hosts.length ? hosts.map((x) => h('a', { href: `#/hosts/${x.id}`, 'data-nav': `#/hosts/${x.id}` },
      h('span', { class: 'dot ' + (x.online ? 'ok' : 'err') }), h('span', { class: 'grow ellipsis' }, x.name),
      x.online && x.info.version && latest_agent && x.info.version !== latest_agent && h('span', { class: 'mini-badge', title: 'Agent update available' }, '↑'),
      h('span', { class: 'count' }, x.apps || ''))) : [h('p', { class: 'dim small pad' }, 'No hosts yet')]));
    document.getElementById('count-hosts').textContent = hosts.length ? `${hosts.filter((x) => x.online).length}/${hosts.length}` : '';
    document.getElementById('count-apps').textContent = apps.length || '';
    const failed = apps.filter((a) => a.last_status === 'failed').length;
    const dc = document.getElementById('count-deployments');
    dc.textContent = failed ? `${failed} failed` : '';
    dc.classList.toggle('red', !!failed);
    markNav();
  } catch {}
}

function markNav() {
  const hash = location.hash || '#/';
  document.querySelectorAll('[data-nav]').forEach((a) => {
    const n = a.dataset.nav;
    a.classList.toggle('on', n === '#/' ? hash === '#/' : hash === n || hash.startsWith(n + '/'));
  });
}

/** Page frame: breadcrumbs, title, subtitle, actions, then content. */
let pageSpin;
function page({ crumbs = [], title, sub, actions }, ...content) {
  clearTimeout(pageSpin);
  ensureLayout();
  markNav();
  document.body.classList.remove('side-open');
  const main = document.getElementById('main');
  main.replaceChildren(h('div', { class: 'wrap' },
    crumbs.length > 0 && h('div', { class: 'crumbs' }, crumbs.map(([label, href], i) => [i > 0 && h('span', { class: 'sep' }, '/'), h('a', { href }, label)])),
    (title || actions) && h('div', { class: 'page-head' }, h('div', {}, h('h1', {}, title), sub && h('div', { class: 'sub' }, sub)), actions && h('div', { class: 'row' }, actions)),
    content));
  window.scrollTo(0, 0);
}

function tabs(base, current, list) {
  return h('div', { class: 'tabs', role: 'tablist' }, list.map(([id, label]) =>
    h('a', { href: `${base}/${id}`, class: id === current ? 'on' : '', role: 'tab', 'aria-selected': String(id === current) }, label)));
}

const section = (title, desc, body, foot, cls = '') => h('div', { class: 'section ' + cls },
  h('div', { class: 's-head' }, h('div', {}, h('h2', {}, title), desc && h('p', { class: 'muted small' }, desc))),
  body && h('div', { class: 's-body' }, body), foot && h('div', { class: 's-foot' }, foot));

const empty = (title, text, action) => h('div', { class: 'empty' }, h('h2', {}, title), text && h('p', {}, text), action);

async function logout() {
  await api('/api/logout', { method: 'POST' }).catch(() => {});
  clearInterval(sideTimer);
  me = null;
  location.hash = '#/login';
}

// ---------- auth ----------

function authPage(setup) {
  clearInterval(sideTimer);
  const user = h('input', { name: 'username', autocomplete: 'username', required: true, value: setup ? 'admin' : '' });
  const pass = h('input', { type: 'password', name: 'password', autocomplete: setup ? 'new-password' : 'current-password', required: true, minlength: setup ? 10 : null });
  const codeIn = h('input', { class: 'mono', required: true, autocomplete: 'off' });
  const btn = h('button', { class: 'btn primary', type: 'submit' }, setup ? 'Create admin account' : 'Sign in');
  const form = h('form', { onsubmit: async (e) => {
    e.preventDefault();
    const r = await act(btn, () => api(setup ? '/api/setup' : '/api/login', { method: 'POST', body: { username: user.value, password: pass.value, code: codeIn.value } }));
    if (r) { me = await api('/api/me'); location.hash = '#/'; }
  } },
  setup && field('Setup code', codeIn, 'Printed in the server logs on first start.'),
  field('Username', user), field('Password', pass, setup ? 'At least 10 characters.' : null), btn);
  $app.replaceChildren(h('div', { class: 'auth' }, h('div', { class: 'box' },
    h('img', { src: '/logo.svg', alt: 'NextPane' }), h('h1', {}, setup ? 'Welcome to NextPane' : 'Sign in to NextPane'),
    h('p', { class: 'muted' }, setup ? 'Create the administrator account.' : 'Your hosts, your apps.'), form)));
  (setup ? codeIn : user).focus();
}

// ---------- command palette ----------

async function palette() {
  if (document.querySelector('dialog.palette') || !me?.user) return;
  const [{ hosts }, { apps }] = await Promise.all([api('/api/hosts'), api('/api/apps')]);
  const items = [
    ...NAV.map(([href, label, ic]) => ({ label, hint: 'Go to', href, ic })),
    ...hosts.map((x) => ({ label: x.name, hint: `Host · ${x.online ? 'online' : 'offline'}`, href: `#/hosts/${x.id}`, ic: 'hosts' })),
    ...apps.map((a) => ({ label: a.spec.name, hint: `App · ${a.host_name}`, href: `#/apps/${a.id}`, ic: 'apps' })),
    ...apps.map((a) => ({ label: `Deploy ${a.spec.name}`, hint: `Action · ${a.host_name}`, ic: 'deployments', run: () => act(null, () => api(`/api/apps/${a.id}/deploy`, { method: 'POST', body: {} }), `Deploying ${a.spec.name}`) })),
    ...apps.map((a) => ({ label: `Logs of ${a.spec.name}`, hint: `Action · ${a.host_name}`, href: `#/apps/${a.id}/logs`, ic: 'activity' })),
    { label: 'Add host', hint: 'Action', ic: 'plus', run: addHostDialog },
    { label: 'New app', hint: 'Action', ic: 'plus', run: () => newAppDialog() },
    { label: 'Sign out', hint: 'Action', ic: 'logout', run: logout },
  ];
  const input = h('input', { class: 'palette-input', placeholder: 'Search hosts, apps, pages, actions…', 'aria-label': 'Search' });
  const list = h('div', { class: 'palette-list', role: 'listbox' });
  let shown = [], sel = 0;
  const go = (it) => { d.close(); if (it.href) location.hash = it.href; else it.run(); };
  const draw = () => {
    const q = input.value.toLowerCase().split(/\s+/).filter(Boolean);
    shown = items.filter((it) => q.every((w) => `${it.label} ${it.hint}`.toLowerCase().includes(w))).slice(0, 50);
    sel = Math.min(sel, Math.max(0, shown.length - 1));
    list.replaceChildren(...(shown.length ? shown.map((it, i) => h('button', { type: 'button', class: 'palette-item' + (i === sel ? ' on' : ''), role: 'option', 'aria-selected': String(i === sel), onclick: () => go(it), onmousemove: () => { if (sel !== i) { sel = i; draw(); } } },
      icon(it.ic), h('span', { class: 'grow ellipsis' }, it.label), h('span', { class: 'dim small' }, it.hint))) : [h('p', { class: 'dim pad' }, 'No results')]));
    list.querySelector('.on')?.scrollIntoView({ block: 'nearest' });
  };
  input.addEventListener('input', () => { sel = 0; draw(); });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') { sel = Math.min(sel + 1, shown.length - 1); draw(); e.preventDefault(); }
    if (e.key === 'ArrowUp') { sel = Math.max(sel - 1, 0); draw(); e.preventDefault(); }
    if (e.key === 'Enter' && shown[sel]) { go(shown[sel]); e.preventDefault(); }
  });
  const d = modal(null, h('div', {}, h('div', { class: 'palette-head' }, icon('search'), input, h('kbd', {}, 'esc')), list), null, 'palette');
  draw();
  input.focus();
}

document.addEventListener('keydown', (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); palette(); }
});

// ---------- shared pieces ----------

function versionBadge(info, latest) {
  if (!info?.version) return null;
  const old = latest && latest !== info.version;
  return h('span', { class: 'badge' + (old ? ' blue' : ''), title: old ? `Agent ${latest} is available` : 'Agent version' }, `agent ${info.version}`, old && ' · update');
}

function hostMeters(m) {
  if (!m) return h('p', { class: 'dim small' }, 'No metrics');
  const appsMem = sum(m.apps, (a) => a.mem), appsCpu = sum(m.apps, (a) => a.cpu);
  const meter = (label, value, parts) => h('div', { class: 'meter' }, h('div', { class: 'row' }, h('span', {}, label), h('span', { class: 'mono' }, value)), bar(parts));
  return h('div', {},
    meter('CPU', pct(m.cpu), [['apps', appsCpu], ['sys', m.cpu - appsCpu]]),
    meter('Memory', `${bytes(m.mem_used)} / ${bytes(m.mem_total)}`, [['apps', (appsMem / m.mem_total) * 100], ['sys', ((m.mem_used - appsMem) / m.mem_total) * 100]]),
    meter('Disk', `${bytes(m.disk_used)} / ${bytes(m.disk_total)}`, [[m.disk_used / m.disk_total > 0.9 ? 'hot' : 'sys', (m.disk_used / m.disk_total) * 100]]));
}

function statCard(label, value, sub, extra) {
  return h('div', {}, h('div', { class: 'stat-label' }, h('span', {}, label), sub && h('span', { class: 'mono small' }, sub)), h('div', { class: 'stat-value' }, value), extra);
}

function appStatus(a) {
  if (a.last_status === 'running') return ['run', 'Deploying'];
  if (!a.online) return ['', 'Host offline'];
  if (!a.metrics || !a.last_sha) return ['', 'Not deployed'];
  if (a.last_status === 'failed') return [a.metrics.running ? 'warn' : 'err', a.metrics.running ? 'Running (last deploy failed)' : 'Failed'];
  return a.metrics.running ? ['ok', 'Running'] : ['err', 'Stopped'];
}

function deploymentsTable(deps, { showApp = false, onOpen, onRedeploy } = {}) {
  const cols = ['Status', showApp && 'App', showApp && 'Host', 'Commit', 'Trigger', 'Started', 'Duration', onRedeploy && ''].filter((c) => c !== false && c !== undefined);
  return h('div', { class: 'table-wrap' }, h('table', {},
    h('thead', {}, h('tr', {}, cols.map((t) => h('th', {}, t)))),
    h('tbody', {}, deps.map((d) => h('tr', { class: 'link', onclick: () => onOpen(d) },
      h('td', {}, h('span', { class: 'row nowrap' }, h('span', { class: 'dot ' + statusDotCls(d.status) }), d.status)),
      showApp && h('td', {}, h('a', { href: `#/apps/${d.app_id}`, onclick: (e) => e.stopPropagation() }, h('strong', {}, d.app))),
      showApp && h('td', { class: 'muted' }, d.host),
      h('td', { class: 'mono' }, short(d.sha)), h('td', { class: 'muted' }, d.trigger), h('td', { class: 'muted nowrap' }, ago(d.started)), h('td', { class: 'mono muted' }, dur(d.started, d.finished) || '…'),
      onRedeploy && h('td', { class: 'num' }, d.sha && d.status !== 'running' && h('button', { class: 'btn sm', title: 'Deploy this commit again (rollback)', onclick: (e) => { e.stopPropagation(); onRedeploy(d, e.currentTarget); } }, 'Redeploy')))))));
}

let logTimer;
async function showLog(container, depId, onDone) {
  clearTimeout(logTimer);
  const d = await api(`/api/deployments/${depId}`);
  const pre = h('pre', { class: 'log' }, d.log || (d.status === 'running' ? 'Running… the full log appears when the deployment finishes.' : '(empty)'));
  container.replaceChildren(h('div', { class: 'section' },
    h('div', { class: 's-head' }, h('h2', { class: 'row' }, h('span', { class: 'dot ' + statusDotCls(d.status) }), `Deployment #${d.id} · ${short(d.sha)} · ${d.status}`),
      h('div', { class: 'row' }, copyBtn(() => d.log), h('button', { class: 'btn ghost sm', onclick: () => container.replaceChildren() }, 'Close'))),
    h('div', { class: 's-body' }, pre)));
  pre.scrollTop = pre.scrollHeight;
  container.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  if (d.status === 'running') logTimer = setTimeout(() => container.isConnected && showLog(container, depId, onDone), 3000);
  else onDone?.();
}

// ---------- overview ----------

async function overview() {
  const body = h('div', {});
  const draw = async () => {
    const [{ hosts, latest_agent }, { apps }, { deployments }] = await Promise.all([api('/api/hosts'), api('/api/apps'), api('/api/deployments?limit=200')]);
    const online = hosts.filter((x) => x.online);
    const running = apps.filter((a) => a.metrics?.running);
    const day = deployments.filter((d) => d.started > Date.now() / 1000 - 86400);
    const failedDay = day.filter((d) => d.status === 'failed').length;
    const ms = online.map((x) => x.metrics).filter(Boolean);
    const memUsed = sum(ms, (m) => m.mem_used), memTotal = sum(ms, (m) => m.mem_total);
    const appsMem = sum(ms, (m) => sum(m.apps, (a) => a.mem));
    const cpuAvg = ms.length ? sum(ms, (m) => m.cpu) / ms.length : 0;

    const alerts = [];
    for (const x of hosts) {
      if (!x.online) alerts.push(['err', `${x.name} is offline`, `last seen ${ago(x.last_seen)}`, `#/hosts/${x.id}`]);
      const m = x.online && x.metrics;
      if (m && m.disk_used / m.disk_total > 0.85) alerts.push(['warn', `${x.name}: disk ${pct((m.disk_used / m.disk_total) * 100)} full`, bytes(m.disk_total - m.disk_used) + ' free', `#/hosts/${x.id}`]);
      if (m && m.mem_used / m.mem_total > 0.9) alerts.push(['warn', `${x.name}: memory ${pct((m.mem_used / m.mem_total) * 100)} used`, '', `#/hosts/${x.id}`]);
      if (x.online && latest_agent && x.info.version && x.info.version !== latest_agent) alerts.push(['info', `${x.name}: agent ${x.info.version} → ${latest_agent} available`, 'update from Hosts', `#/hosts`]);
    }
    for (const a of apps) {
      if (a.last_status === 'failed') alerts.push(['err', `${a.spec.name}: last deployment failed`, a.host_name, `#/apps/${a.id}`]);
      else if (a.online && a.last_sha && a.metrics && !a.metrics.running && a.last_status !== 'running') alerts.push(['err', `${a.spec.name} is stopped`, a.host_name, `#/apps/${a.id}`]);
      if (a.metrics?.running && a.spec.memory_mb && a.metrics.mem > a.spec.memory_mb * KB * KB * 0.9) alerts.push(['warn', `${a.spec.name}: memory at ${pct((a.metrics.mem / (a.spec.memory_mb * KB * KB)) * 100)} of its limit`, a.host_name, `#/apps/${a.id}`]);
    }
    const order = { err: 0, warn: 1, info: 2 };
    alerts.sort((a, b) => order[a[0]] - order[b[0]]);

    body.replaceChildren(
      h('div', { class: 'stats' },
        statCard('Hosts online', h('span', {}, `${online.length}`, h('small', {}, `of ${hosts.length}`)), null, bar([['apps', hosts.length ? (online.length / hosts.length) * 100 : 0]])),
        statCard('Apps running', h('span', {}, `${running.length}`, h('small', {}, `of ${apps.length}`)), null, bar([['apps', apps.length ? (running.length / apps.length) * 100 : 0]])),
        statCard('Deployments · 24h', h('span', {}, `${day.length}`, failedDay ? h('small', { class: 'red' }, `${failedDay} failed`) : h('small', {}, 'none failed')), null,
          h('div', { class: 'legend' }, h('span', {}, h('i', { class: 'apps' }), `${day.filter((d) => d.status === 'success').length} succeeded`), h('span', {}, h('i', { class: 'hot' }), `${failedDay} failed`))),
        statCard('Fleet', h('span', {}, pct(cpuAvg), h('small', {}, 'avg CPU')), `${bytes(memUsed)} / ${bytes(memTotal)}`,
          [bar([['apps', memTotal ? (appsMem / memTotal) * 100 : 0], ['sys', memTotal ? ((memUsed - appsMem) / memTotal) * 100 : 0]]),
            h('div', { class: 'legend' }, h('span', {}, h('i', { class: 'apps' }), `apps ${bytes(appsMem)}`), h('span', {}, h('i', { class: 'sys' }), `system ${bytes(memUsed - appsMem)}`))])),
      h('div', { class: 'cols' },
        h('div', { class: 'stack' },
          section(alerts.length ? `Needs attention · ${alerts.length}` : 'Needs attention', null, alerts.length
            ? h('div', { class: 'alerts' }, alerts.slice(0, 12).map(([lvl, text, sub2, href]) => h('a', { class: 'alert-row', href }, h('span', { class: 'dot ' + (lvl === 'info' ? 'blue' : lvl) }), h('span', { class: 'grow' }, text), h('span', { class: 'dim small' }, sub2), icon('arrow', 14))))
            : h('div', { class: 'all-good' }, icon('check', 18), 'Everything is healthy.'), null, 'flush'),
          h('div', {}, h('div', { class: 'section-title' }, h('h2', {}, 'Recent deployments'), h('a', { href: '#/deployments', class: 'muted small' }, 'View all →')),
            deployments.length ? deploymentsTable(deployments.slice(0, 8), { showApp: true, onOpen: (d) => (location.hash = `#/apps/${d.app_id}`) }) : empty('No deployments yet', 'Create an app on a host and deploy it.'))),
        h('div', { class: 'stack' },
          h('div', { class: 'section-title' }, h('h2', {}, 'Hosts'), h('a', { href: '#/hosts', class: 'muted small' }, 'View all →')),
          hosts.length ? hosts.map((x) => h('a', { class: 'card', href: `#/hosts/${x.id}` },
            h('div', { class: 'card-head' }, h('div', { class: 'row' }, h('span', { class: 'dot ' + (x.online ? 'ok' : 'err') }), h('span', { class: 'card-title' }, x.name)), h('span', { class: 'dim small' }, `${x.apps} app${x.apps === 1 ? '' : 's'}`)),
            x.online ? hostMeters(x.metrics) : h('p', { class: 'dim small' }, `Offline · seen ${ago(x.last_seen)}`)))
            : empty('No hosts yet', 'Install the agent on a Linux server.', h('button', { class: 'btn primary', onclick: addHostDialog }, 'Add host')))));
  };
  page({ title: 'Overview', sub: 'Your fleet at a glance', actions: [h('button', { class: 'btn', onclick: () => newAppDialog() }, icon('plus', 14), 'New app'), h('button', { class: 'btn primary', onclick: addHostDialog }, icon('plus', 14), 'Add host')] }, body);
  await draw();
  every(5000, () => draw().catch(() => {}));
}

// ---------- hosts ----------

async function hostsPage() {
  const filter = h('input', { type: 'search', placeholder: 'Filter hosts…', class: 'filter', 'aria-label': 'Filter hosts' });
  const grid = h('div', {});
  const updAll = h('button', { class: 'btn', hidden: true });
  let data = await api('/api/hosts');
  const draw = () => {
    const { hosts, latest_agent } = data;
    const q = filter.value.toLowerCase();
    const shown = hosts.filter((x) => `${x.name} ${x.info.hostname || ''} ${x.info.os || ''}`.toLowerCase().includes(q));
    const outdated = hosts.filter((x) => x.online && latest_agent && x.info.version && x.info.version !== latest_agent);
    updAll.hidden = !outdated.length;
    updAll.textContent = `Update ${outdated.length} agent${outdated.length === 1 ? '' : 's'} to ${latest_agent}`;
    updAll.onclick = async () => {
      if (!(await confirmDialog('Update agents', `Update ${outdated.map((x) => x.name).join(', ')} to ${latest_agent}? Apps keep running during the update.`, 'Update all', false))) return;
      await act(updAll, async () => {
        const res = await Promise.allSettled(outdated.map((x) => api(`/api/hosts/${x.id}/update`, { method: 'POST' })));
        const failed = res.filter((r) => r.status === 'rejected');
        if (failed.length) throw new Error(`${failed.length} update(s) failed: ${failed[0].reason.message}`);
      }, 'Agents are updating, they reconnect in a few seconds');
    };
    grid.replaceChildren(!hosts.length
      ? empty('No hosts yet', 'Install the agent on a Linux server to start deploying apps.', h('button', { class: 'btn primary', onclick: addHostDialog }, 'Add your first host'))
      : h('div', { class: 'grid' }, shown.map((x) => h('a', { class: 'card', href: `#/hosts/${x.id}` },
        h('div', { class: 'card-head' }, h('div', { class: 'row' }, h('span', { class: 'dot ' + (x.online ? 'ok' : 'err'), title: x.online ? 'online' : 'offline' }), h('span', { class: 'card-title' }, x.name)), versionBadge(x.info, latest_agent)),
        h('p', { class: 'muted small' }, [x.info.os, x.info.arch, x.info.cpus && `${x.info.cpus} vCPU`, `${x.apps} app${x.apps === 1 ? '' : 's'}`].filter(Boolean).join(' · ') || 'waiting for agent', !x.online && ` · seen ${ago(x.last_seen)}`),
        x.online ? hostMeters(x.metrics) : h('p', { class: 'dim small' }, 'Offline')))));
  };
  filter.addEventListener('input', draw);
  page({ title: 'Hosts', sub: `${data.hosts.length} host${data.hosts.length === 1 ? '' : 's'} · ${data.hosts.filter((x) => x.online).length} online`, actions: [updAll, h('button', { class: 'btn primary', onclick: addHostDialog }, icon('plus', 14), 'Add host')] },
    h('div', { class: 'toolbar' }, filter), grid);
  draw();
  every(5000, async () => { try { data = await api('/api/hosts'); draw(); } catch {} });
}

async function addHostDialog() {
  const name = h('input', { placeholder: 'production', value: 'default' });
  const out = h('div', { class: 'stack' });
  const create = h('button', { class: 'btn primary', type: 'button', onclick: () => act(create, async () => {
    const [{ key }, sys] = await Promise.all([api('/api/enroll-keys', { method: 'POST', body: { name: name.value } }), api('/api/system')]);
    const pin = location.protocol === 'https:' && sys.tls_pin;
    create.remove();
    out.replaceChildren(
      h('p', { class: 'muted small' }, 'This key is shown only once. It can enroll any number of hosts; revoke it in Settings when done.'),
      h('h3', {}, 'One-line install (Debian, Ubuntu, RHEL, Alma, Rocky…)'),
      pin
        ? code(`curl -fsSLk --pinnedpubkey sha256//${pin.spki} ${location.origin}/install.sh | sudo NEXTPANE_SERVER=${wsOrigin()}/agent/ws NEXTPANE_FINGERPRINT=${pin.cert} NEXTPANE_TOKEN=${key} sh`)
        : code(`curl -fsSL ${location.origin}/install.sh | sudo NEXTPANE_SERVER=${wsOrigin()}/agent/ws NEXTPANE_TOKEN=${key} sh`),
      pin && h('p', { class: 'dim small' }, 'The panel uses a self-signed certificate: the agent and curl trust only its fingerprint, shown in the panel logs at startup.'),
      location.protocol === 'http:' && h('p', { class: 'dim small' }, 'Plain HTTP: the token and environment variables travel unencrypted. Use only on a trusted network, or enable NEXTPANE_TLS.'),
      h('h3', {}, 'Ansible (group_vars)'),
      code(`nextpane_server_url: "${wsOrigin()}/agent/ws"\nnextpane_enroll_token: "${key}"${pin ? `\nnextpane_server_fingerprint: "${pin.cert}"` : ''}`),
      h('h3', {}, 'Manual install: /etc/nextpane/agent.env'),
      code(`NEXTPANE_SERVER=${wsOrigin()}/agent/ws\nNEXTPANE_TOKEN=${key}${pin ? `\nNEXTPANE_FINGERPRINT=${pin.cert}` : ''}`),
      h('p', { class: 'dim small' }, 'The host appears in the sidebar as soon as the agent connects.'));
  }) }, 'Create enrollment key');
  modal('Add a host', h('div', { class: 'stack' }, h('p', { class: 'muted' }, 'The agent opens one outbound WebSocket to this panel. No inbound port is needed on the host.'), field('Key name', name), out), create);
}

function hostStats(host) {
  const hist = host.history || [];
  const m = host.metrics;
  if (!host.online || !m) return h('div', { class: 'empty' }, host.online ? 'Waiting for the first metrics…' : `Host offline · last seen ${ago(host.last_seen)}`);
  const s = (x, k) => sum(x.apps, (a) => a[k]);
  const appsCpu = s(m, 'cpu'), appsMem = s(m, 'mem'), appsDisk = s(m, 'disk');
  const lg = (a, b) => h('div', { class: 'legend' }, h('span', {}, h('i', { class: 'apps' }), `apps ${a}`), h('span', {}, h('i', { class: 'sys' }), `system ${b}`));
  return h('div', { class: 'stats' },
    statCard('CPU', h('span', {}, pct(m.cpu), h('small', {}, `load ${m.load.map((l) => l.toFixed(2)).join(' ')}`)), `${host.info.cpus} vCPU`,
      [spark(hist.map((x) => x.cpu), 100, hist.map((x) => s(x, 'cpu'))), lg(pct(appsCpu), pct(m.cpu - appsCpu))]),
    statCard('Memory', h('span', {}, bytes(m.mem_used), h('small', {}, `of ${bytes(m.mem_total)}`)), m.swap_total ? `swap ${bytes(m.swap_used)}` : null,
      [spark(hist.map((x) => x.mem_used), m.mem_total, hist.map((x) => s(x, 'mem'))), lg(bytes(appsMem), bytes(m.mem_used - appsMem))]),
    statCard('Disk', h('span', {}, bytes(m.disk_used), h('small', {}, `of ${bytes(m.disk_total)}`)), pct((m.disk_used / m.disk_total) * 100),
      [bar([['apps', (appsDisk / m.disk_total) * 100], ['sys', ((m.disk_used - appsDisk) / m.disk_total) * 100]]), lg(bytes(appsDisk), bytes(m.disk_used - appsDisk))]),
    statCard('Network', h('span', {}, `${bytes(m.net_rx)}/s`, h('small', {}, `↑ ${bytes(m.net_tx)}/s`)), `up ${uptime(m.uptime)}`,
      spark(hist.map((x) => x.net_rx), Math.max(1, ...hist.map((x) => Math.max(x.net_rx, x.net_tx))), hist.map((x) => x.net_tx))));
}

function appsTable(apps, { showHost = false } = {}) {
  if (!apps.length) return null;
  return h('div', { class: 'table-wrap' }, h('table', {},
    h('thead', {}, h('tr', {}, ['App', showHost && 'Host', 'Status', 'Branch', 'CPU', 'Memory', 'Disk', 'Last deploy'].filter(Boolean).map((t) => h('th', { class: ['CPU', 'Memory', 'Disk'].includes(t) ? 'num' : '' }, t)))),
    h('tbody', {}, apps.map((a) => {
      const m = a.metrics;
      const [cls, label] = appStatus(a);
      const lim = a.spec.memory_mb ? ` / ${a.spec.memory_mb} MB` : '';
      return h('tr', { class: 'link', onclick: () => (location.hash = `#/apps/${a.id}`) },
        h('td', {}, h('div', { class: 'row nowrap' }, h('a', { href: `#/apps/${a.id}` }, h('strong', {}, a.spec.name)), a.spec.port && h('span', { class: 'dim mono small' }, `:${a.spec.port}`), m?.discord_js ? h('span', { class: 'badge discord', title: `discord.js ${m.discord_js}` }, 'Discord bot') : m?.framework && h('span', { class: 'badge' }, m.framework))),
        showHost && h('td', { class: 'muted' }, a.host_name),
        h('td', {}, h('span', { class: 'row nowrap' }, h('span', { class: 'dot ' + cls }), label)),
        h('td', { class: 'mono small muted nowrap' }, `${a.spec.branch} @ ${short(a.last_sha)}`),
        h('td', { class: 'num mono' }, m?.running ? pct(m.cpu) : '—'),
        h('td', { class: 'num mono nowrap' }, m?.running ? bytes(m.mem) + lim : '—'),
        h('td', { class: 'num mono' }, m ? bytes(m.disk) : '—'),
        h('td', { class: 'muted nowrap' }, a.last_status ? h('span', { class: a.last_status === 'failed' ? 'badge red' : '' }, `${a.last_status} · ${ago(a.last_deploy)}`) : 'never'));
    }))));
}

const APP_DEFAULTS = { name: '', repo: '', branch: 'main', install_cmd: '', build_cmd: '', start_cmd: '', port: 3000, cpu_percent: null, memory_mb: null, cache_paths: ['.next/cache'], auto_deploy: true, firewall: [] };

function specForm(spec, isNew) {
  const s = { ...APP_DEFAULTS, ...spec };
  const f = {
    name: h('input', { value: s.name, required: true, pattern: '[a-z0-9][a-z0-9-]{0,23}', placeholder: 'my-site', class: 'mono' }),
    repo: h('input', { value: s.repo, required: true, placeholder: 'git@github.com:me/site.git or https://…', class: 'mono' }),
    branch: h('input', { value: s.branch, required: true, class: 'mono' }),
    port: h('input', { type: 'number', min: 1, max: 65535, value: s.port ?? '' }),
    install_cmd: h('input', { value: s.install_cmd, placeholder: 'auto (npm ci / pnpm / yarn / bun)', class: 'mono' }),
    build_cmd: h('input', { value: s.build_cmd, placeholder: 'auto (run build if present)', class: 'mono' }),
    start_cmd: h('input', { value: s.start_cmd, placeholder: 'npm start', class: 'mono' }),
    cpu_percent: h('input', { type: 'number', min: 1, value: s.cpu_percent ?? '', placeholder: 'unlimited' }),
    memory_mb: h('input', { type: 'number', min: 16, value: s.memory_mb ?? '', placeholder: 'unlimited' }),
    cache_paths: h('textarea', { class: 'mono', rows: 2 }, (s.cache_paths || []).join('\n')),
    auto_deploy: h('input', { type: 'checkbox', checked: s.auto_deploy }),
  };
  const num = (el) => (el.value === '' ? null : Number(el.value));
  const read = () => ({
    ...s,
    name: f.name.value.trim(), repo: f.repo.value.trim(), branch: f.branch.value.trim(), port: num(f.port),
    install_cmd: f.install_cmd.value.trim(), build_cmd: f.build_cmd.value.trim(), start_cmd: f.start_cmd.value.trim(),
    cpu_percent: num(f.cpu_percent), memory_mb: num(f.memory_mb), auto_deploy: f.auto_deploy.checked,
    cache_paths: f.cache_paths.value.split(/[\n,]/).map((x) => x.trim()).filter(Boolean),
  });
  const tpl = isNew && select([['web', 'Web app (Next.js, Express…)'], ['discord', 'Discord bot (discord.js)']], 'web', 'Type');
  if (tpl) {
    tpl.classList.remove('auto');
    tpl.onchange = () => {
      const bot = tpl.value === 'discord';
      f.port.value = bot ? '' : 3000;
      f.cache_paths.value = bot ? '' : '.next/cache';
      f.start_cmd.placeholder = bot ? 'node index.js' : 'npm start';
      f.build_cmd.placeholder = bot ? 'auto (or e.g. node deploy-commands.js)' : 'auto (run build if present)';
    };
  }
  const el = h('div', { class: 'form' },
    tpl && field('Type', tpl, 'Discord bots need no port: put the token in DISCORD_TOKEN, NextPane then shows the bot, its servers and slash commands.'),
    isNew && field('Name', f.name, 'Lowercase, used for the systemd unit and folders.'),
    field('Git repository', f.repo, 'Any provider. Private repos: generate a deploy key after creating the app.'),
    field('Branch', f.branch), field('Port', f.port, 'Exported as PORT.'),
    field('Install command', f.install_cmd), field('Build command', f.build_cmd), field('Start command', f.start_cmd),
    field('CPU limit (%)', f.cpu_percent, '100 = one full core.'), field('Memory limit (MB)', f.memory_mb, 'Hard limit, the app is restarted if it exceeds it.'),
    field('Cache paths', f.cache_paths, 'Relative to the app, one per line. Wiped by “Clear cache”.'),
    h('label', { class: 'check full' }, f.auto_deploy, h('span', {}, 'Deploy automatically on every new commit')));
  return { el, read };
}

async function newAppDialog(hostId) {
  const { hosts } = hostId ? { hosts: [] } : await api('/api/hosts');
  if (!hostId && !hosts.length) return toast('Add a host first.', true);
  const hostSel = !hostId && select(hosts.map((x) => [String(x.id), `${x.name}${x.online ? '' : ' (offline)'}`]), String(hosts[0]?.id), 'Host');
  const form = specForm({}, true);
  const btn = h('button', { class: 'btn primary', type: 'button', onclick: () => act(btn, async () => {
    const r = await api(`/api/hosts/${hostId || hostSel.value}/apps`, { method: 'POST', body: form.read() });
    d.close();
    refreshSide();
    location.hash = `#/apps/${r.id}/settings`;
  }, 'App created. Add environment variables or a deploy key, then deploy.') }, 'Create app');
  const d = modal('New app', h('div', { class: 'stack' }, hostSel && field('Host', hostSel), form.el), btn, 'wide');
}

async function hostPage(id, tab = 'overview') {
  if (tab === 'apps') tab = 'overview';
  let host = await api(`/api/hosts/${id}`);
  const live = h('div', {});
  const body = h('div', {});
  const head = h('div', {});
  const drawHead = () => {
    const i = host.info;
    const canUpdate = host.online && host.latest_agent && i.version && host.latest_agent !== i.version;
    const upd = h('button', { class: 'btn', onclick: () => act(upd, () => api(`/api/hosts/${id}/update`, { method: 'POST' }), (r) => r.output) }, `Update agent to ${host.latest_agent}`);
    head.replaceChildren(h('div', { class: 'page-head' },
      h('div', {}, h('h1', {}, host.name), h('div', { class: 'sub' },
        h('span', { class: 'row' }, h('span', { class: 'dot ' + (host.online ? 'ok' : 'err') }), host.online ? 'Online' : `Offline · ${ago(host.last_seen)}`),
        [i.hostname, i.ip, i.os, i.kernel && `kernel ${i.kernel}`, i.arch].filter(Boolean).map((x) => h('span', {}, x)),
        i.firewall && h('span', { class: 'badge' }, `firewall: ${i.firewall}`), versionBadge(i, host.latest_agent))),
      h('div', { class: 'row' }, canUpdate && upd, h('button', { class: 'btn primary', onclick: () => newAppDialog(id) }, icon('plus', 14), 'New app'))));
  };
  const drawLive = () => {
    drawHead();
    if (tab !== 'overview') return;
    const apps = host.apps.map((a) => ({ ...a, online: host.online, metrics: host.online ? (host.metrics?.apps || []).find((m) => m.name === a.spec.name) : null }));
    live.replaceChildren(hostStats(host), h('div', { class: 'section-title' }, h('h2', {}, 'Apps')),
      appsTable(apps) || empty('No apps on this host', 'Link a Git repository to deploy a Node.js app.', h('button', { class: 'btn primary', onclick: () => newAppDialog(id) }, 'New app')));
  };

  if (tab === 'keys') {
    const { keys: library } = await api('/api/ssh-keys');
    const name = h('input', { placeholder: 'laptop' });
    const key = h('textarea', { class: 'mono', placeholder: 'ssh-ed25519 AAAA… you@laptop', rows: 3 });
    const keep = h('input', { type: 'checkbox', checked: true });
    const add = h('button', { class: 'btn primary', type: 'button', onclick: () => act(add, async () => {
      const k = { name: name.value, pubkey: key.value };
      await api(`/api/hosts/${id}/ssh-keys`, { method: 'POST', body: k });
      if (keep.checked) await api('/api/ssh-keys', { method: 'POST', body: k });
      render();
    }, 'Key added') }, 'Add key');
    const installed = new Set(host.ssh_keys.map((k) => k.pubkey.trim()));
    const avail = library.filter((k) => !installed.has(k.pubkey.trim()));
    body.append(h('div', { class: 'stack' },
      h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'SSH keys'), h('p', { class: 'muted small' }, `Installed in ~nextpane/.ssh/authorized_keys to log into the host: ssh nextpane@${host.info.ip || host.info.hostname || 'host'}. Git does not use them: private repositories need the app's deploy key (app Settings).`))),
        host.ssh_keys.length ? h('table', {}, h('tbody', {}, host.ssh_keys.map((k) => h('tr', {},
          h('td', {}, h('strong', {}, k.name || 'key')), h('td', { class: 'mono small muted' }, k.pubkey.slice(0, 60) + '…'),
          h('td', { class: 'num' }, h('button', { class: 'btn sm', onclick: async (e) => {
            const b = e.currentTarget;
            if (await confirmDialog('Remove key', `Remove “${k.name}” from this host?`, 'Remove')) act(b, () => api(`/api/hosts/${id}/ssh-keys/${k.id}`, { method: 'DELETE' }).then(render));
          } }, 'Remove')))))) : h('div', { class: 's-body muted' }, 'No keys yet.')),
      avail.length > 0 && h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'From the key library'), h('p', { class: 'muted small' }, 'Saved keys not yet on this host. Manage the library in Settings.'))),
        h('table', {}, h('tbody', {}, avail.map((k) => h('tr', {},
          h('td', {}, h('strong', {}, k.name || 'key')), h('td', { class: 'mono small muted' }, k.pubkey.slice(0, 60) + '…'),
          h('td', { class: 'num' }, h('button', { class: 'btn sm', onclick: (e) => act(e.currentTarget, () => api(`/api/hosts/${id}/ssh-keys`, { method: 'POST', body: { name: k.name, pubkey: k.pubkey } }).then(render), 'Key added') }, 'Add'))))))),
      section('Add a key', 'Public key only (the .pub file). Never paste a private key.', h('div', { class: 'form' }, field('Name', name), h('div', { class: 'full' }, field('Public key', key)),
        h('label', { class: 'check full' }, keep, h('span', {}, 'Save to the key library to reuse it on other hosts'))), [h('span', {}), add])));
  } else if (tab === 'settings') {
    const name = h('input', { value: host.name });
    const save = h('button', { class: 'btn primary', type: 'button', onclick: () => act(save, () => api(`/api/hosts/${id}`, { method: 'PATCH', body: { name: name.value } }).then(() => { refreshSide(); render(); }), 'Saved') }, 'Save');
    const reset = h('button', { class: 'btn', type: 'button', onclick: async () => {
      if (await confirmDialog('Allow re-enrollment', 'The next agent that connects with this machine-id and a valid enrollment key takes over this host. Use it after reinstalling the agent.', 'Allow', false))
        act(reset, () => api(`/api/hosts/${id}`, { method: 'PATCH', body: { reset_identity: true } }), 'Re-enrollment allowed');
    } }, 'Allow re-enrollment');
    const del = h('button', { class: 'btn danger', type: 'button', onclick: async () => {
      if (await confirmDialog('Delete host', 'Removes the host and its apps from NextPane. Nothing is deleted on the server itself; uninstall the agent there.', 'Delete host'))
        act(del, async () => { await api(`/api/hosts/${id}`, { method: 'DELETE' }); refreshSide(); location.hash = '#/hosts'; });
    } }, 'Delete host');
    const i = host.info;
    body.append(h('div', { class: 'stack' },
      section('Display name', null, name, [h('span', { class: 'dim small' }, 'Shown in the sidebar and notifications.'), save]),
      section('System', null, h('dl', { class: 'kv' }, [['Hostname', i.hostname], ['OS', i.os], ['Kernel', i.kernel], ['Architecture', i.arch], ['CPU', i.cpus && `${i.cpus} vCPU`], ['Firewall', i.firewall], ['Agent', i.version], ['Installed as', i.install]]
        .map(([k, v]) => [h('dt', {}, k), h('dd', { class: 'mono' }, v || '—')]))),
      section('Agent identity', 'Each host proves its identity with a secret issued at enrollment.', null, [h('span', {}), reset]),
      section('Delete host', null, null, [h('span', { class: 'small muted' }, 'This cannot be undone.'), del], 'danger')));
  }

  page({ crumbs: [['Hosts', '#/hosts'], [host.name, `#/hosts/${id}`]] }, head, tabs(`#/hosts/${id}`, tab, [['overview', 'Overview'], ['keys', `SSH keys${host.ssh_keys.length ? ` · ${host.ssh_keys.length}` : ''}`], ['settings', 'Settings']]), live, body);
  drawLive();
  every(5000, async () => { try { host = await api(`/api/hosts/${id}`); drawLive(); } catch {} });
}

// ---------- apps ----------

async function appsPage() {
  const q = h('input', { type: 'search', placeholder: 'Filter by name, repo, host…', class: 'filter', 'aria-label': 'Filter apps' });
  const status = select([['', 'All statuses'], ['Running', 'Running'], ['Stopped', 'Stopped'], ['Failed', 'Failed'], ['Deploying', 'Deploying'], ['Not deployed', 'Not deployed'], ['Host offline', 'Host offline']], '', 'Status');
  let data = await api('/api/apps');
  const hosts = [...new Map(data.apps.map((a) => [a.host_id, a.host_name])).entries()];
  const hostSel = select([['', 'All hosts'], ...hosts.map(([i, n]) => [String(i), n])], '', 'Host');
  const out = h('div', {});
  const draw = () => {
    const w = q.value.toLowerCase();
    const shown = data.apps.filter((a) => `${a.spec.name} ${a.spec.repo} ${a.host_name}`.toLowerCase().includes(w)
      && (!status.value || appStatus(a)[1].startsWith(status.value) || (status.value === 'Failed' && a.last_status === 'failed'))
      && (!hostSel.value || String(a.host_id) === hostSel.value));
    out.replaceChildren(data.apps.length ? (appsTable(shown, { showHost: true }) || empty('No matching apps')) : empty('No apps yet', 'Link a Git repository to a host.', h('button', { class: 'btn primary', onclick: () => newAppDialog() }, 'New app')));
  };
  [q, status, hostSel].forEach((el) => el.addEventListener('input', draw));
  page({ title: 'Apps', sub: `${data.apps.length} app${data.apps.length === 1 ? '' : 's'} on ${hosts.length} host${hosts.length === 1 ? '' : 's'}`, actions: h('button', { class: 'btn primary', onclick: () => newAppDialog() }, icon('plus', 14), 'New app') },
    h('div', { class: 'toolbar' }, q, status, hostSel), out);
  draw();
  every(5000, async () => { try { data = await api('/api/apps'); draw(); } catch {} });
}

async function appPage(id, tab = 'deployments') {
  let app = await api(`/api/apps/${id}`);
  const s = app.spec;
  const head = h('div', {});
  const body = h('div', {});
  const run = (btn, fn, msg) => act(btn, fn, msg).then(() => refresh());
  const depList = h('div', {});
  const openLog = h('div', {});

  const redeploy = async (d, btn) => {
    if (await confirmDialog('Redeploy', `Deploy commit ${short(d.sha)} of ${s.name} again? Use it to roll back to this version.`, 'Redeploy', false))
      run(btn, () => api(`/api/apps/${id}/deploy`, { method: 'POST', body: { sha: d.sha } }), `Deploying ${short(d.sha)}`);
  };
  const drawDeployments = () => depList.replaceChildren(app.deployments.length
    ? deploymentsTable(app.deployments, { onOpen: (d) => showLog(openLog, d.id, refresh), onRedeploy: redeploy })
    : empty('No deployments yet', app.online ? 'Press Deploy to build and start the app.' : 'The host is offline.'));

  const refresh = async () => {
    try { app = await api(`/api/apps/${id}`); } catch { return; }
    const m = app.metrics;
    const hist = app.history || [];
    const deploying = app.deployments[0]?.status === 'running';
    const deploy = h('button', { class: 'btn primary' + (deploying ? ' busy' : ''), disabled: !app.online || deploying, onclick: () => run(deploy, () => api(`/api/apps/${id}/deploy`, { method: 'POST', body: {} }), 'Deployment started') }, deploying ? 'Deploying…' : 'Deploy');
    const action = (a, label) => { const b = h('button', { class: 'btn', disabled: !app.online, onclick: () => run(b, () => api(`/api/apps/${id}/action`, { method: 'POST', body: { action: a } }), `${label}: done`) }, label); return b; };
    const cache = h('button', { class: 'btn', disabled: !app.online, onclick: async () => {
      if (await confirmDialog('Clear cache', `Delete ${s.cache_paths.join(', ') || 'nothing (no cache paths configured)'} and restart ${s.name}?`, 'Clear cache', false)) run(cache, () => api(`/api/apps/${id}/cache`, { method: 'POST' }), 'Cache cleared');
    } }, 'Clear cache');
    const [cls, label] = appStatus({ ...app, last_status: app.deployments[0]?.status });
    const memLimit = s.memory_mb ? s.memory_mb * KB * KB : 0;
    const peak = (k) => Math.max(1, ...hist.map((x) => x[k])) * 1.2;
    head.replaceChildren(h('div', { class: 'page-head' },
      h('div', {}, h('h1', { class: 'row' }, h('span', { class: 'dot big ' + cls }), s.name), h('div', { class: 'sub' },
        h('span', { class: 'mono' }, s.repo.replace(/^(https?:\/\/)[^@/]*@/, '$1')), h('span', { class: 'badge mono' }, `${s.branch} @ ${short(app.last_sha)}`),
        s.port && (app.host_ip ? appLink(app.host_ip, s.port) : h('span', { class: 'mono' }, `:${s.port}`)), m?.framework && h('span', { class: 'badge' }, m.framework),
        h('a', { href: `#/hosts/${app.host_id}`, class: 'badge' }, icon('hosts', 12), app.host_name), !app.online && h('span', { class: 'badge red' }, 'host offline'))),
      h('div', { class: 'row' }, m?.running ? action('stop', 'Stop') : action('start', 'Start'), action('restart', 'Restart'), cache, deploy)),
    h('div', { class: 'stats' },
      statCard('Status', label, s.auto_deploy ? 'auto-deploy' : 'manual', h('p', { class: 'dim small' }, app.deployments[0] ? `last deploy ${ago(app.deployments[0].started)}` : 'never deployed')),
      statCard('CPU', pct(m?.cpu), s.cpu_percent ? `limit ${s.cpu_percent}%` : 'no limit', spark(hist.map((x) => x.cpu), s.cpu_percent || peak('cpu'))),
      statCard('Memory', bytes(m?.mem), s.memory_mb ? `limit ${s.memory_mb} MB` : 'no limit',
        [spark(hist.map((x) => x.mem), memLimit || peak('mem')), memLimit ? bar([[(m?.mem || 0) > memLimit * 0.9 ? 'hot' : 'apps', ((m?.mem || 0) / memLimit) * 100]]) : null]),
      statCard('Disk', bytes(m?.disk), null, h('p', { class: 'dim small' }, 'code, dependencies and builds'))));
    if (tab === 'deployments') drawDeployments();
  };

  if (tab === 'deployments') {
    const hook = `${location.origin}${app.webhook_path}`;
    body.append(h('div', { class: 'stack' }, depList, openLog,
      section('Deploy hook', 'POST this URL from any Git provider (GitHub, GitLab, Gitea, Forgejo…) to deploy instantly. Pushes to other branches are ignored.', code(hook))));
  } else if (tab === 'logs') {
    const pre = h('pre', { class: 'log tall' }, 'Loading…');
    const lines = select([100, 300, 1000, 3000].map((n) => [String(n), `${n} lines`]), '300', 'Lines');
    const grep = h('input', { type: 'search', placeholder: 'Filter lines…', class: 'filter', 'aria-label': 'Filter log lines' });
    let raw = '';
    const show = () => {
      const w = grep.value.toLowerCase();
      const stick = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 20;
      pre.textContent = (w ? raw.split('\n').filter((l) => l.toLowerCase().includes(w)).join('\n') : raw) || '(no output yet)';
      if (stick) pre.scrollTop = pre.scrollHeight;
    };
    const load = async () => {
      try { raw = (await api(`/api/apps/${id}/logs?lines=${lines.value}`)).output; show(); } catch (e) { pre.textContent = e.message; }
    };
    lines.onchange = load;
    grep.oninput = show;
    const follow = h('input', { type: 'checkbox', checked: true });
    body.append(h('div', { class: 'stack' }, h('div', { class: 'toolbar' }, grep, h('span', { class: 'dim grow mono small' }, `journalctl -u nextpane-app-${s.name}`), h('label', { class: 'check small muted' }, follow, 'Live'), lines,
      h('button', { class: 'btn sm', onclick: load }, 'Refresh'), h('button', { class: 'btn sm', onclick: () => download(`${s.name}.log`, raw) }, 'Download')), pre));
    await load();
    pre.scrollTop = pre.scrollHeight;
    every(5000, () => follow.checked && load());
  } else if (tab === 'env') {
    body.append(await envEditor(id, s.name));
  } else if (tab === 'discord') {
    body.append(discordTab(app));
  } else if (tab === 'firewall') {
    body.append(firewallEditor(app, refresh));
  } else if (tab === 'settings') {
    const form = specForm(s, false);
    const save = h('button', { class: 'btn primary', type: 'button', onclick: () => act(save, () => api(`/api/apps/${id}`, { method: 'PUT', body: form.read() }), (r) => r.output || 'Saved').then(refresh) }, 'Save & apply');
    const keyBox = h('div', {}, app.deploy_key ? code(app.deploy_key) : h('p', { class: 'muted small' }, 'No deploy key yet.'));
    const gen = h('button', { class: 'btn', type: 'button', disabled: !app.online, onclick: () => act(gen, async () => {
      const r = await api(`/api/apps/${id}/deploy-key`, { method: 'POST' });
      keyBox.replaceChildren(code(r.data));
    }, 'Deploy key ready') }, app.deploy_key ? 'Show key' : 'Generate deploy key');
    const { hosts } = await api('/api/hosts');
    const dupHost = select(hosts.map((x) => [String(x.id), `${x.name}${x.id === app.host_id ? ' (this host)' : ''}`]), String(hosts.find((x) => x.id !== app.host_id)?.id ?? app.host_id), 'Target host');
    dupHost.classList.remove('auto');
    const dupName = h('input', { class: 'mono', value: `${s.name}-copy`.slice(0, 24) });
    const dupEnv = h('input', { type: 'checkbox', checked: true });
    const dup = h('button', { class: 'btn', type: 'button', onclick: () => act(dup, async () => {
      const r = await api(`/api/hosts/${dupHost.value}/apps`, { method: 'POST', body: { ...s, name: dupName.value.trim() } });
      if (dupEnv.checked) {
        const { vars } = await api(`/api/apps/${id}/env`);
        if (vars.length) await api(`/api/apps/${r.id}/env`, { method: 'PUT', body: { vars, apply: false } });
      }
      refreshSide();
      location.hash = `#/apps/${r.id}`;
    }, 'App duplicated, deploy it when ready') }, 'Duplicate');
    const del = h('button', { class: 'btn danger', type: 'button', onclick: async () => {
      if (!(await confirmDialog('Delete app', `Stops ${s.name}, removes its systemd unit, firewall rules, env file and code from the host.`, 'Delete app'))) return;
      act(del, async () => {
        try { await api(`/api/apps/${id}`, { method: 'DELETE' }); } catch (e) {
          if (!(await confirmDialog('Host unreachable', `${e.message}\n\nDelete it from NextPane anyway? Files stay on the host.`, 'Delete anyway'))) return;
          await api(`/api/apps/${id}?force=true`, { method: 'DELETE' });
        }
        refreshSide();
        location.hash = `#/hosts/${app.host_id}`;
      });
    } }, 'Delete app');
    body.append(h('div', { class: 'stack' },
      section('Build & runtime', null, form.el, [h('span', { class: 'dim small' }, 'Limits and commands apply immediately (the app restarts).'), save]),
      section('Deploy key', 'For private repositories: an SSH key generated on the host (the private half never leaves it, you never paste a private key). Add the public key below as a read-only deploy key in the repository (GitHub: Settings > Deploy keys; GitLab: Settings > Repository > Deploy keys) and use the SSH clone URL (git@…).', keyBox, [h('span', {}), gen]),
      section('Duplicate', 'Create the same app (settings, firewall rules and optionally environment variables) on another host, e.g. to scale out or to build a staging copy. Private repos need a deploy key on the new host.',
        h('div', { class: 'form' }, field('Target host', dupHost), field('Name', dupName), h('label', { class: 'check full' }, dupEnv, h('span', {}, 'Copy environment variables'))), [h('span', {}), dup]),
      section('Delete app', null, null, [h('span', { class: 'small muted' }, 'This cannot be undone.'), del], 'danger')));
  }

  page({ crumbs: [['Hosts', '#/hosts'], [app.host_name, `#/hosts/${app.host_id}`], [s.name, `#/apps/${id}`]] }, head,
    tabs(`#/apps/${id}`, tab, [['deployments', 'Deployments'], ['logs', 'Logs'], app.discord && ['discord', 'Discord'], ['env', 'Environment'], ['firewall', `Firewall${s.firewall.length ? ` · ${s.firewall.length}` : ''}`], ['settings', 'Settings']].filter(Boolean)), body);
  await refresh();
  every(5000, refresh);
}

async function envEditor(id, name) {
  const { vars } = await api(`/api/apps/${id}/env`);
  const rows = h('div', {});
  const addRow = (k = '', v = '') => {
    const key = h('input', { class: 'mono', value: k, placeholder: 'KEY', 'aria-label': 'Name' });
    const val = h('input', { class: 'mono', type: 'password', value: v, placeholder: 'value', autocomplete: 'off', 'aria-label': 'Value' });
    const row = h('div', { class: 'env-row' }, key, val,
      h('button', { class: 'btn ghost sm', type: 'button', onclick: (e) => { val.type = val.type === 'password' ? 'text' : 'password'; e.currentTarget.textContent = val.type === 'password' ? 'Show' : 'Hide'; } }, 'Show'),
      h('button', { class: 'btn ghost sm', type: 'button', 'aria-label': 'Remove', onclick: () => row.remove() }, 'Remove'));
    row._read = () => ({ key: key.value.trim(), value: val.value });
    rows.append(row);
  };
  vars.forEach((v) => addRow(v.key, v.value));
  if (!vars.length) addRow();
  const read = () => [...rows.children].map((r) => r._read()).filter((v) => v.key);
  const apply = h('input', { type: 'checkbox', checked: true });
  const save = h('button', { class: 'btn primary', type: 'button', onclick: () => act(save, () => api(`/api/apps/${id}/env`, { method: 'PUT', body: { vars: read(), apply: apply.checked } }), 'Environment saved') }, 'Save');
  const paste = h('button', { class: 'btn', type: 'button', onclick: () => {
    const ta = h('textarea', { class: 'mono', rows: 10, placeholder: 'KEY=value\nOTHER="quoted value"' });
    const imp = h('button', { class: 'btn primary', value: 'ok', onclick: () => {
      for (const line of ta.value.split('\n')) {
        const m = line.match(/^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)\s*$/);
        if (m) addRow(m[1], m[2].replace(/^(['"])(.*)\1$/, '$2'));
      }
      [...rows.children].filter((r) => !r._read().key).forEach((r) => r.remove());
    } }, 'Import');
    modal('Paste a .env file', ta, imp);
  } }, 'Paste .env');
  const exp = h('button', { class: 'btn', type: 'button', onclick: () => {
    const pw = h('input', { type: 'password', autocomplete: 'current-password', required: true });
    const pass = h('input', { type: 'password', autocomplete: 'new-password', minlength: 12, placeholder: 'leave empty for a plain .env' });
    const go = h('button', { class: 'btn primary', type: 'button', onclick: () => act(go, async () => {
      const r = await api(`/api/apps/${id}/env/export`, { method: 'POST', body: { password: pw.value, passphrase: pass.value } });
      download(r.filename, r.content);
      d.close();
    }, 'Exported (recorded in the activity log)') }, 'Export');
    const d = modal('Export environment', h('div', { class: 'stack' },
      h('p', { class: 'muted' }, 'Exports are recorded in the activity log. Prefer an encrypted export: plain files leak easily through downloads folders, backups and chat.'),
      field('Your password', pw), field('Encryption passphrase (recommended)', pass, 'Decrypt with: openssl enc -d -aes-256-cbc -pbkdf2 -iter 600000 -md sha256 -a -A -in FILE')), go);
  } }, 'Export…');
  return h('div', { class: 'section' },
    h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'Environment variables'), h('p', { class: 'muted small' }, `Encrypted at rest on the panel, written to /etc/nextpane/apps/${name}.env (root, 0600) on the host. Available during build and at runtime.`)), h('div', { class: 'row' }, paste, exp)),
    h('div', { class: 's-body' }, rows, h('button', { class: 'btn sm', type: 'button', onclick: () => addRow() }, '+ Add variable')),
    h('div', { class: 's-foot' }, h('label', { class: 'check small muted' }, apply, 'Apply now (restarts the app)'), save));
}

// ---------- discord bots ----------

const PERMS = [
  ['0', 'None (slash commands only)'],
  [String([6, 10, 11, 14, 15, 16, 18, 38].reduce((s, b) => s + 2 ** b, 0)), 'Common: view, send, embeds, files, history, reactions, threads'],
  ['8', 'Administrator'],
];
const cdn = (kind, id, hash, size) => /^\d+$/.test(id || '') && /^(a_)?[0-9a-f]+$/.test(hash || '') ? `https://cdn.discordapp.com/${kind}/${id}/${hash}.png?size=${size}` : null;
const roundImg = (src, label, cls) => (src ? h('img', { class: cls, src, alt: '' }) : h('span', { class: cls }, (label || '?')[0].toUpperCase()));

function discordTab(app) {
  const box = h('div', { class: 'stack' }, h('p', { class: 'muted' }, 'Asking Discord…'));
  const load = async () => {
    let d;
    try { d = await api(`/api/apps/${app.id}/discord`); } catch (e) { return box.replaceChildren(empty('Cannot load the bot', e.message)); }
    const a = d.application;
    const problems = [];
    if (!d.token_var) problems.push(['err', `No bot token. Add ${d.token_vars[0]} in the Environment tab (${d.token_vars.slice(1).join(', ')} also work), then restart.`]);
    else if (d.token_valid === false) problems.push(['err', `Discord rejects the token in ${d.token_var}. Reset it in the Developer Portal (Bot → Reset Token) and update the variable.`]);
    if (d.error) problems.push(['warn', `Discord API: ${d.error}`]);
    if (!d.online) problems.push(['warn', 'Host offline: process status and logs are unavailable.']);
    else if (!d.running) problems.push(['err', 'The bot process is not running. Check the Logs tab, then Start or Deploy.']);
    (d.diagnostics || []).forEach((x) => problems.push([x.level, x.advice, x.line]));
    const healthy = d.running && d.token_valid && !problems.some((p) => p[0] === 'err');
    const [cls, label] = healthy ? ['ok', 'Online'] : d.running ? ['warn', 'Running with problems'] : ['err', 'Down'];

    const perms = select(PERMS, PERMS[1][0], 'Permissions');
    perms.classList.remove('auto');
    const invite = h('div', {});
    const drawInvite = () => invite.replaceChildren(code(`https://discord.com/oauth2/authorize?client_id=${a.id}&scope=bot+applications.commands&permissions=${perms.value}`));
    perms.onchange = drawInvite;
    if (a) drawInvite();

    const del = (what, url, label) => async (e) => {
      const b = e.currentTarget;
      if (await confirmDialog(label, what, label)) act(b, () => api(url, { method: 'DELETE' }), 'Done').then(load);
    };
    const intent = (on, name) => h('span', { class: 'badge' + (on ? '' : ' off') }, on ? '✓ ' : '✕ ', name);
    const refresh = h('button', { class: 'btn sm', onclick: (e) => act(e.currentTarget, load) }, 'Refresh');

    box.replaceChildren(
      a ? h('div', { class: 'bot-head' }, roundImg(cdn('avatars', a.bot.id, a.bot.avatar, 96), a.bot.username || a.name, 'avatar-lg'),
        h('div', { class: 'grow' }, h('h2', {}, a.bot.username || a.name), h('div', { class: 'muted small row' }, h('span', { class: 'mono' }, a.id), a.public === false && h('span', { class: 'badge' }, 'private bot'), d.token_var && h('span', {}, `token from ${d.token_var}`))),
        h('a', { class: 'btn sm', href: `https://discord.com/developers/applications/${a.id}/bot`, target: '_blank', rel: 'noopener noreferrer' }, 'Developer Portal ↗'), refresh)
        : h('div', { class: 'row' }, h('span', { class: 'grow' }), refresh),
      h('div', { class: 'stats' },
        statCard('Bot status', h('span', { class: 'row' }, h('span', { class: 'dot big ' + cls }), label), null, d.ready_line && h('p', { class: 'dim small ellipsis', title: d.ready_line }, d.ready_line)),
        statCard('discord.js', d.discord_js || '—', null, h('p', { class: 'dim small' }, d.discord_js ? 'detected in the deployed release' : 'not detected (deploy first)')),
        statCard('Servers', a ? String(a.guild_count ?? d.guilds.length) : '—'),
        statCard('Slash commands', a ? String(d.commands.length) : '—', null, h('p', { class: 'dim small' }, 'global commands'))),
      section(problems.length ? `Diagnostics · ${problems.length}` : 'Diagnostics', 'From the Discord API and the last 300 log lines.', problems.length
        ? h('div', { class: 'alerts' }, problems.map(([lvl, text, line]) => h('div', { class: 'alert-row' }, h('span', { class: 'dot ' + lvl }), h('div', { class: 'grow' }, h('div', {}, text), line && h('div', { class: 'mono dim small ellipsis' }, line)))))
        : h('div', { class: 'all-good' }, icon('check', 18), 'No problem detected.'), null, 'flush'),
      a && section('Privileged intents', 'Enabled in the Developer Portal. If the code asks for an intent that is off here, discord.js stops with “Used disallowed intents”.',
        h('div', { class: 'row' }, intent(d.intents.presence, 'Presence'), intent(d.intents.members, 'Server members'), intent(d.intents.message_content, 'Message content'))),
      a && section('Invite link', 'Add the bot to a server with these permissions.', h('div', { class: 'stack' }, field('Permissions', perms), invite)),
      a && h('div', { class: 'section' }, h('div', { class: 's-head' }, h('h2', {}, `Servers · ${d.guilds.length}`)),
        d.guilds.length ? h('table', {}, h('tbody', {}, d.guilds.map((g) => h('tr', {},
          h('td', {}, h('div', { class: 'row nowrap' }, roundImg(cdn('icons', g.id, g.icon, 64), g.name, 'guild-icon'), h('strong', {}, g.name), g.owner && h('span', { class: 'badge' }, 'owner'))),
          h('td', { class: 'muted small nowrap' }, g.members != null ? `${g.members} members` : ''), h('td', { class: 'mono dim small' }, g.id),
          h('td', { class: 'num' }, h('button', { class: 'btn sm', onclick: del(`Make the bot leave “${g.name}”? It needs a new invite to come back.`, `/api/apps/${app.id}/discord/guilds/${g.id}`, 'Leave server') }, 'Leave'))))))
          : h('div', { class: 's-body muted' }, 'The bot is in no server yet: use the invite link.')),
      a && h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, `Global slash commands · ${d.commands.length}`), h('p', { class: 'muted small' }, 'Registered by your code (REST put on applications.commands). Delete stale ones here; a bot that re-registers on start brings them back.'))),
        d.commands.length ? h('table', {}, h('tbody', {}, d.commands.map((c) => h('tr', {},
          h('td', { class: 'mono' }, `${c.type === 1 || c.type == null ? '/' : ''}${c.name}`), h('td', { class: 'muted small' }, c.description || (c.type === 2 ? 'user context menu' : c.type === 3 ? 'message context menu' : '')),
          h('td', { class: 'num' }, h('button', { class: 'btn sm', onclick: del(`Delete the command “${c.name}” from Discord?`, `/api/apps/${app.id}/discord/commands/${c.id}`, 'Delete command') }, 'Delete'))))))
          : h('div', { class: 's-body muted' }, 'No global commands.')));
  };
  load();
  return box;
}

function firewallEditor(app, refresh) {
  const rows = h('div', {});
  const addRow = (r = { port: app.spec.port || '', proto: 'tcp', source: '' }) => {
    const port = h('input', { type: 'number', min: 1, max: 65535, value: r.port, 'aria-label': 'Port' });
    const proto = h('select', { 'aria-label': 'Protocol' }, ['tcp', 'udp'].map((p) => h('option', { value: p, selected: p === r.proto }, p.toUpperCase())));
    const src = h('input', { class: 'mono', value: r.source || '', placeholder: 'anywhere (or 10.0.0.0/8, 203.0.113.4)', 'aria-label': 'Source' });
    const row = h('div', { class: 'fw-row' }, port, proto, src, h('button', { class: 'btn ghost sm', type: 'button', onclick: () => row.remove() }, 'Remove'));
    row._read = () => ({ port: Number(port.value), proto: proto.value, source: src.value.trim() || null });
    rows.append(row);
  };
  (app.spec.firewall || []).forEach(addRow);
  const save = h('button', { class: 'btn primary', type: 'button', onclick: () => act(save, () => api(`/api/apps/${app.id}`, { method: 'PUT', body: { ...app.spec, firewall: [...rows.children].map((r) => r._read()).filter((r) => r.port) } }), (r) => r.output || 'Rules applied').then(refresh) }, 'Save & apply');
  return h('div', { class: 'section' },
    h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'Firewall rules'), h('p', { class: 'muted small' }, 'Inbound ports opened on the host for this app. NextPane detects ufw, firewalld, nftables or iptables and manages only its own rules (removed with the app).'))),
    h('div', { class: 's-body' }, h('div', { class: 'fw-row dim small' }, h('span', {}, 'Port'), h('span', {}, 'Protocol'), h('span', {}, 'Allowed source'), h('span', {})), rows,
      h('button', { class: 'btn sm', type: 'button', onclick: () => addRow() }, '+ Add rule')),
    h('div', { class: 's-foot' }, h('span', { class: 'dim small' }, 'Tip: behind a reverse proxy on the same host, you usually need no rule for the app port at all.'), save));
}

// ---------- deployments ----------

async function deploymentsPage() {
  const status = select([['', 'All statuses'], ['running', 'Running'], ['success', 'Succeeded'], ['failed', 'Failed']], '', 'Status');
  const q = h('input', { type: 'search', placeholder: 'Filter by app, host, commit…', class: 'filter', 'aria-label': 'Filter deployments' });
  const list = h('div', {});
  const openLog = h('div', {});
  let data = { deployments: [] };
  const draw = () => {
    const w = q.value.toLowerCase();
    const shown = data.deployments.filter((d) => `${d.app} ${d.host} ${d.sha || ''} ${d.trigger}`.toLowerCase().includes(w));
    list.replaceChildren(shown.length ? deploymentsTable(shown, { showApp: true, onOpen: (d) => showLog(openLog, d.id) }) : empty('No deployments', status.value || w ? 'Nothing matches these filters.' : 'Deploy an app to see it here.'));
  };
  const load = async () => { data = await api(`/api/deployments?limit=200&status=${status.value}`); draw(); };
  status.addEventListener('input', load);
  q.addEventListener('input', draw);
  page({ title: 'Deployments', sub: 'Every build across all hosts, newest first' }, h('div', { class: 'toolbar' }, q, status), openLog, list);
  await load();
  every(5000, () => load().catch(() => {}));
}

// ---------- activity ----------

async function activityPage() {
  const { entries } = await api('/api/audit');
  const q = h('input', { type: 'search', placeholder: 'Filter activity…', class: 'filter', 'aria-label': 'Filter activity' });
  const out = h('div', {});
  const kind = (a) => (a.startsWith('env') || a.includes('delete') ? 'red' : a.includes('deploy') ? 'blue' : '');
  const draw = () => {
    const w = q.value.toLowerCase();
    const shown = entries.filter((e) => `${e.action} ${e.detail}`.toLowerCase().includes(w));
    out.replaceChildren(shown.length ? h('div', { class: 'table-wrap' }, h('table', {}, h('thead', {}, h('tr', {}, ['When', 'Action', 'Details'].map((t) => h('th', {}, t)))),
      h('tbody', {}, shown.map((e) => h('tr', {}, h('td', { class: 'muted small nowrap', title: new Date(e.ts * 1000).toLocaleString() }, ago(e.ts)), h('td', {}, h('span', { class: 'badge mono ' + kind(e.action) }, e.action)), h('td', { class: 'small' }, e.detail))))))
      : empty('Nothing found'));
  };
  q.addEventListener('input', draw);
  page({ title: 'Activity', sub: 'Audit log: enrollments, deployments, secrets, keys, updates (last 200 events)' }, h('div', { class: 'toolbar' }, q), out);
  draw();
}

// ---------- settings ----------

async function settingsPage() {
  const [sys, { keys }, settings, { keys: library }] = await Promise.all([api('/api/system'), api('/api/enroll-keys'), api('/api/settings'), api('/api/ssh-keys')]);
  const libName = h('input', { placeholder: 'laptop' });
  const libKey = h('textarea', { class: 'mono', placeholder: 'ssh-ed25519 AAAA… you@laptop', rows: 3 });
  const libAdd = h('button', { class: 'btn primary', type: 'button', onclick: () => act(libAdd, () => api('/api/ssh-keys', { method: 'POST', body: { name: libName.value, pubkey: libKey.value } }).then(render), 'Key saved') }, 'Save key');
  const docker = sys.install === 'docker';
  const upd = h('button', { class: 'btn primary', disabled: !sys.update_available || docker, onclick: async () => {
    if (await confirmDialog('Update NextPane', `Install ${sys.latest}? The panel restarts and will be unavailable for a few seconds. Agents reconnect automatically.`, 'Update', false))
      act(upd, () => api('/api/system/update', { method: 'POST' }), (r) => r.output).then(() => setTimeout(() => location.reload(), 8000));
  } }, sys.update_available ? `Update to ${sys.latest}` : 'Up to date');

  const n = settings.notifications;
  const url = h('input', { class: 'mono', value: n.url, autocomplete: 'off' });
  const fmt = select([['slack', 'Slack / Mattermost'], ['discord', 'Discord'], ['matrix', 'Matrix'], ['generic', 'Generic JSON']], n.format || 'slack', 'Format');
  fmt.classList.remove('auto');
  const room = h('input', { class: 'mono', value: n.room || '', placeholder: '!abcdef:matrix.org', autocomplete: 'off' });
  const mtoken = h('input', { class: 'mono', type: 'password', autocomplete: 'off', placeholder: settings.token_set ? 'saved · leave empty to keep it' : 'syt_…' });
  const matrixBox = h('div', { class: 'form full' },
    field('Room ID', room, 'Room settings → Advanced. Invite the bot account to the room first.'),
    field('Access token', mtoken, 'Of the account that posts (Element: Settings → Help & About). Stored encrypted.'));
  const urlField = field('Webhook URL', url, 'Stored encrypted. Leave empty to disable notifications.');
  const syncFmt = () => {
    const mx = fmt.value === 'matrix';
    matrixBox.hidden = !mx;
    urlField.firstChild.textContent = mx ? 'Homeserver URL' : 'Webhook URL';
    url.placeholder = mx ? 'https://matrix.org' : 'https://hooks.slack.com/services/… or https://discord.com/api/webhooks/…';
  };
  fmt.addEventListener('change', syncFmt);
  syncFmt();
  const labels = { deploy_failed: 'Deployment failed', deploy_success: 'Deployment succeeded', host_offline: 'Host offline (after 1 minute)', host_online: 'Host back online' };
  const evs = Object.fromEntries(settings.events.map((e) => [e, h('input', { type: 'checkbox', checked: n.url ? n.events.includes(e) : e !== 'deploy_success' })]));
  const readN = () => ({ notifications: { url: url.value.trim(), format: fmt.value, room: room.value.trim(), token: mtoken.value.trim(), events: Object.entries(evs).filter(([, c]) => c.checked).map(([e]) => e) } });
  const saveN = h('button', { class: 'btn primary', type: 'button', onclick: () => act(saveN, () => api('/api/settings', { method: 'PUT', body: readN() }), 'Notifications saved') }, 'Save');
  const testN = h('button', { class: 'btn', type: 'button', onclick: () => act(testN, () => api('/api/settings/test', { method: 'POST', body: readN() }), 'Test message sent') }, 'Send test');

  const cur = h('input', { type: 'password', autocomplete: 'current-password' });
  const nw = h('input', { type: 'password', autocomplete: 'new-password', minlength: 10 });
  const pw = h('button', { class: 'btn primary', type: 'button', onclick: () => act(pw, async () => { await api('/api/password', { method: 'POST', body: { current: cur.value, new: nw.value } }); me = null; location.hash = '#/login'; }, 'Password changed, sign in again') }, 'Change password');

  page({ title: 'Settings', sub: `NextPane ${sys.version}` }, h('div', { class: 'stack' },
    section('Server updates', `Version ${sys.version} · installed as ${sys.install}`,
      h('p', { class: 'muted' },
        !sys.updates_enabled ? 'Updates are disabled: set NEXTPANE_RELEASE_URL in the server configuration.'
          : sys.error ? `Cannot check for updates: ${sys.error}`
            : docker ? `Latest: ${sys.latest}. Running in Docker: update with “docker compose pull && docker compose up -d”.`
              : `Latest release: ${sys.latest}.`),
      [h('span', { class: 'dim small' }, 'Agents are updated from the Hosts page (one by one or all at once).'), upd]),
    section('Notifications', 'Get a message in Slack, Discord, Mattermost, Matrix or any JSON webhook when something happens.',
      h('div', { class: 'form' }, field('Format', fmt), h('div', {}), h('div', { class: 'full' }, urlField), matrixBox,
        h('div', {}, h('span', { class: 'label' }, 'Events'), h('div', { class: 'checks' }, Object.entries(evs).map(([e, c]) => h('label', { class: 'check small' }, c, labels[e] || e))))),
      [testN, saveN]),
    h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'Enrollment keys'), h('p', { class: 'muted small' }, 'Let new agents register. Revoking a key does not disconnect hosts that already enrolled.')), h('button', { class: 'btn', type: 'button', onclick: addHostDialog }, 'New key')),
      keys.length ? h('table', {}, h('tbody', {}, keys.map((k) => h('tr', {}, h('td', {}, h('strong', {}, k.name)), h('td', { class: 'muted' }, `created ${ago(k.created)}`),
        h('td', { class: 'num' }, h('button', { class: 'btn sm', onclick: async (e) => {
          const b = e.currentTarget;
          if (await confirmDialog('Revoke key', `Revoke “${k.name}”? New hosts can no longer enroll with it.`, 'Revoke')) act(b, () => api(`/api/enroll-keys/${k.id}`, { method: 'DELETE' }).then(render));
        } }, 'Revoke')))))) : h('div', { class: 's-body muted' }, 'No active keys.')),
    h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'SSH key library'), h('p', { class: 'muted small' }, 'Public keys saved once, then added to any host from its SSH keys tab. Forgetting a key here does not remove it from hosts.'))),
      library.length ? h('table', {}, h('tbody', {}, library.map((k) => h('tr', {}, h('td', {}, h('strong', {}, k.name || 'key')), h('td', { class: 'mono small muted' }, k.pubkey.slice(0, 60) + '…'),
        h('td', { class: 'num' }, h('button', { class: 'btn sm', onclick: async (e) => {
          const b = e.currentTarget;
          if (await confirmDialog('Forget key', `Remove “${k.name}” from the library? Hosts keep it.`, 'Forget')) act(b, () => api(`/api/ssh-keys/${k.id}`, { method: 'DELETE' }).then(render));
        } }, 'Forget')))))) : h('div', { class: 's-body muted' }, 'No saved keys.'),
      h('div', { class: 's-body' }, h('div', { class: 'form' }, field('Name', libName), h('div', { class: 'full' }, field('Public key', libKey)))),
      h('div', { class: 's-foot' }, h('span', { class: 'dim small' }, 'Public key only (the .pub file).'), libAdd)),
    section('Password', null, h('div', { class: 'form' }, field('Current password', cur), field('New password', nw, 'At least 10 characters. Signs out every session.')), [h('span', {}), pw])));
}

// ---------- router ----------

const routes = [
  [/^#\/hosts$/, hostsPage],
  [/^#\/hosts\/(\d+)(?:\/(\w+))?$/, hostPage],
  [/^#\/apps$/, appsPage],
  [/^#\/apps\/(\d+)(?:\/(\w+))?$/, appPage],
  [/^#\/deployments$/, deploymentsPage],
  [/^#\/activity$/, activityPage],
  [/^#\/settings$/, settingsPage],
  [/^#\/$/, overview],
];

async function render() {
  timers.forEach(clearInterval);
  timers = [];
  clearTimeout(logTimer);
  document.querySelectorAll('dialog').forEach((d) => d.close());
  const hash = location.hash || '#/';
  if (hash === '#/login' || hash === '#/setup') {
    me = await api('/api/me');
    if (me.user) { location.hash = '#/'; return; }
    return authPage(me.needs_setup);
  }
  if (!me?.user) {
    me = await api('/api/me');
    if (!me.user) { location.hash = me.needs_setup ? '#/setup' : '#/login'; return; }
  }
  for (const [re, view] of routes) {
    const m = hash.match(re);
    if (!m) continue;
    // Only slow loads get the circle, so fast navigation does not flicker.
    clearTimeout(pageSpin);
    pageSpin = setTimeout(() => document.getElementById('main')?.replaceChildren(h('div', { class: 'page-loading', role: 'status', 'aria-label': 'Loading' }, h('span', { class: 'spinner' }))), 250);
    try {
      await view(...m.slice(1).filter((x) => x !== undefined));
    } catch (e) {
      if (!e.auth) page({ title: 'Something went wrong' }, empty('Something went wrong', e.message, h('a', { class: 'btn', href: '#/' }, 'Back to overview')));
    }
    clearTimeout(pageSpin); // views that redirect never call page()
    return;
  }
  page({ title: 'Page not found' }, empty('Page not found', null, h('a', { class: 'btn', href: '#/' }, 'Back to overview')));
}

window.addEventListener('hashchange', render);
render();
