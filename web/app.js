// NextPane web UI. No build step, no dependencies. DOM is built with h(), never innerHTML.

const $app = document.getElementById('app');
let timers = [];

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
  const t = h('div', { class: 'toast' + (err ? ' err' : ''), role: err ? 'alert' : 'status' }, msg);
  document.getElementById('toasts').append(t);
  setTimeout(() => t.remove(), err ? 8000 : 4000);
}

/** Run an async action from a button: disables it, toasts the result. */
async function act(btn, fn, okMsg) {
  if (btn) btn.disabled = true;
  try {
    const r = await fn();
    if (okMsg) toast(typeof okMsg === 'function' ? okMsg(r) : okMsg);
    return r;
  } catch (e) {
    if (!e.auth) toast(e.message, true);
  } finally {
    if (btn) btn.disabled = false;
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
const code = (text) => h('div', { class: 'code' }, text, copyBtn(text));

function field(label, input, hint) {
  return h('label', { class: 'field' }, h('span', {}, label), input, hint && h('small', {}, hint));
}

function bar(parts) {
  return h('div', { class: 'bar' }, parts.map(([cls, v]) => {
    const i = h('i', { class: cls });
    i.style.width = `${Math.max(0, Math.min(100, v))}%`;
    return i;
  }));
}

/** Sparkline of one or two series (values 0..max). */
function spark(values, max, second) {
  const n = values.length;
  const wrap = svg('svg', { class: 'spark', viewBox: '0 0 100 36', preserveAspectRatio: 'none', 'aria-hidden': 'true' });
  if (n < 2) return wrap;
  const pts = (vs) => vs.map((v, i) => `${(i / (n - 1)) * 100},${34 - (Math.min(v, max) / (max || 1)) * 32}`);
  const p = pts(values);
  wrap.append(svg('path', { class: 'a', d: `M0,36L${p.join('L')}L100,36Z` }), svg('path', { class: 'l', d: `M${p.join('L')}` }));
  if (second) wrap.append(svg('path', { class: 'l2', d: `M${pts(second).join('L')}` }));
  return wrap;
}

function modal(title, body, actions) {
  // Enter in a field must not close the dialog: only explicit submit buttons (confirm) do.
  const d = h('dialog', {}, h('form', { method: 'dialog', onsubmit: (e) => !e.submitter && e.preventDefault() },
    h('div', { class: 'd-body' }, h('h2', {}, title), body),
    h('div', { class: 'd-foot' }, h('button', { class: 'btn', type: 'button', onclick: () => d.close('cancel') }, 'Close'), actions || [])));
  d.addEventListener('close', () => d.remove());
  document.body.append(d);
  d.showModal();
  return d;
}

function confirmDialog(title, text, label = 'Confirm', danger = true) {
  return new Promise((resolve) => {
    const d = modal(title, h('p', { class: 'muted' }, text), h('button', { class: 'btn ' + (danger ? 'danger' : 'primary'), value: 'ok' }, label));
    d.addEventListener('close', () => resolve(d.returnValue === 'ok'));
  });
}

function download(filename, content) {
  const a = h('a', { href: URL.createObjectURL(new Blob([content], { type: 'application/octet-stream' })), download: filename });
  document.body.append(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

// ---------- layout ----------

function shell(crumbs, ...content) {
  const nav = [h('a', { href: '#/' }, 'Hosts'), h('a', { href: '#/settings' }, 'Settings'),
    h('button', { onclick: async () => { await api('/api/logout', { method: 'POST' }); location.hash = '#/login'; } }, 'Sign out')];
  const trail = [h('a', { href: '#/', class: 'brand' }, h('img', { src: '/logo.svg', alt: '' }), h('span', {}, 'NextPane'))];
  for (const [label, href] of crumbs) trail.push(h('span', { class: 'sep' }, '/'), h('a', { href }, label));
  $app.replaceChildren(h('header', { class: 'top' }, h('div', { class: 'in' }, h('div', { class: 'crumbs' }, trail), h('nav', { class: 'right' }, nav))), h('main', {}, content));
}

function tabs(base, current, list) {
  return h('div', { class: 'tabs', role: 'tablist' }, list.map(([id, label]) =>
    h('a', { href: `${base}/${id}`, class: id === current ? 'on' : '', role: 'tab', 'aria-selected': String(id === current) }, label)));
}

// ---------- auth ----------

function authPage(setup) {
  const user = h('input', { name: 'username', autocomplete: 'username', required: true, value: setup ? 'admin' : '' });
  const pass = h('input', { type: 'password', name: 'password', autocomplete: setup ? 'new-password' : 'current-password', required: true, minlength: setup ? 10 : null });
  const codeIn = h('input', { class: 'mono', required: true, autocomplete: 'off' });
  const btn = h('button', { class: 'btn primary', type: 'submit' }, setup ? 'Create admin account' : 'Sign in');
  const form = h('form', { onsubmit: async (e) => {
    e.preventDefault();
    const r = await act(btn, () => api(setup ? '/api/setup' : '/api/login', { method: 'POST', body: { username: user.value, password: pass.value, code: codeIn.value } }));
    if (r) { location.hash = '#/'; render(); }
  } },
  setup && field('Setup code', codeIn, 'Printed in the server logs on first start.'),
  field('Username', user), field('Password', pass, setup ? 'At least 10 characters.' : null), btn);
  $app.replaceChildren(h('div', { class: 'auth' }, h('div', { class: 'box' },
    h('img', { src: '/logo.svg', alt: 'NextPane' }), h('h1', {}, setup ? 'Welcome to NextPane' : 'Sign in to NextPane'),
    h('p', { class: 'muted' }, setup ? 'Create the administrator account.' : 'Your hosts, your apps.'), form)));
  (setup ? codeIn : user).focus();
}

// ---------- overview ----------

function hostMeters(m) {
  if (!m) return h('p', { class: 'dim small' }, 'No metrics yet');
  const appsMem = m.apps.reduce((s, a) => s + a.mem, 0);
  const appsCpu = m.apps.reduce((s, a) => s + a.cpu, 0);
  const meter = (label, value, parts) => h('div', { class: 'meter' }, h('div', { class: 'row' }, h('span', {}, label), h('span', { class: 'mono' }, value)), bar(parts));
  return h('div', {},
    meter('CPU', pct(m.cpu), [['apps', appsCpu], ['sys', m.cpu - appsCpu]]),
    meter('Memory', `${bytes(m.mem_used)} / ${bytes(m.mem_total)}`, [['apps', (appsMem / m.mem_total) * 100], ['sys', ((m.mem_used - appsMem) / m.mem_total) * 100]]),
    meter('Disk', `${bytes(m.disk_used)} / ${bytes(m.disk_total)}`, [[m.disk_used / m.disk_total > 0.9 ? 'hot' : 'sys', (m.disk_used / m.disk_total) * 100]]));
}

function versionBadge(info, latest) {
  if (!info?.version) return null;
  const old = latest && latest !== info.version;
  return h('span', { class: 'badge' + (old ? ' blue' : ''), title: old ? `Agent ${latest} is available` : 'Agent version' }, `agent ${info.version}`, old && ' · update');
}

async function overview() {
  const draw = async () => {
    const { hosts, latest_agent } = await api('/api/hosts');
    const add = h('button', { class: 'btn primary', onclick: addHostDialog }, 'Add host');
    shell([], h('div', { class: 'page-head' }, h('div', {}, h('h1', {}, 'Hosts'), h('div', { class: 'sub' }, `${hosts.length} host${hosts.length === 1 ? '' : 's'} · ${hosts.filter((x) => x.online).length} online`)), add),
      hosts.length === 0
        ? h('div', { class: 'empty' }, h('h2', {}, 'No hosts yet'), h('p', {}, 'Install the agent on a Linux server to start deploying apps.'), h('button', { class: 'btn primary', onclick: addHostDialog }, 'Add your first host'))
        : h('div', { class: 'grid' }, hosts.map((x) => h('a', { class: 'card', href: `#/hosts/${x.id}` },
          h('div', { class: 'card-head' }, h('div', { class: 'row' }, h('span', { class: 'dot ' + (x.online ? 'ok' : 'err'), title: x.online ? 'online' : 'offline' }), h('span', { class: 'card-title' }, x.name)), versionBadge(x.info, latest_agent)),
          h('p', { class: 'muted small' }, [x.info.os, x.info.arch, `${x.apps} app${x.apps === 1 ? '' : 's'}`].filter(Boolean).join(' · ') || 'waiting for agent', !x.online && ` · seen ${ago(x.last_seen)}`),
          hostMeters(x.online ? x.metrics : null)))));
  };
  await draw();
  every(10000, () => !document.querySelector('dialog') && draw().catch(() => {}));
}

async function addHostDialog() {
  const name = h('input', { placeholder: 'production', value: 'default' });
  const out = h('div', { class: 'stack' });
  const create = h('button', { class: 'btn primary', type: 'button', onclick: () => act(create, async () => {
    const { key } = await api('/api/enroll-keys', { method: 'POST', body: { name: name.value } });
    create.remove();
    out.replaceChildren(
      h('p', { class: 'muted small' }, 'This key is shown only once. It can enroll any number of hosts; revoke it in Settings when done.'),
      h('h3', {}, 'One-line install (Debian, Ubuntu, RHEL, Alma, Rocky...)'),
      code(`curl -fsSL ${location.origin}/install.sh | sudo NEXTPANE_SERVER=${wsOrigin()}/agent/ws NEXTPANE_TOKEN=${key} sh`),
      h('h3', {}, 'Ansible (group_vars)'),
      code(`nextpane_server_url: "${wsOrigin()}/agent/ws"\nnextpane_enroll_token: "${key}"`),
      h('h3', {}, 'Manual install: /etc/nextpane/agent.env'),
      code(`NEXTPANE_SERVER=${wsOrigin()}/agent/ws\nNEXTPANE_TOKEN=${key}`),
      h('p', { class: 'dim small' }, 'The host appears here as soon as the agent connects.'));
  }) }, 'Create enrollment key');
  modal('Add a host', h('div', { class: 'stack' }, h('p', { class: 'muted' }, 'The agent opens one outbound WebSocket to this panel. No inbound port is needed on the host.'), field('Key name', name), out), create);
}

// ---------- host ----------

function statCard(label, value, sub, extra) {
  return h('div', {}, h('div', { class: 'stat-label' }, h('span', {}, label), sub && h('span', { class: 'mono small' }, sub)), h('div', { class: 'stat-value' }, value), extra);
}

function hostStats(host) {
  const hist = host.history || [];
  const m = host.metrics;
  if (!host.online || !m) return h('div', { class: 'empty' }, host.online ? 'Waiting for the first metrics…' : `Host offline · last seen ${ago(host.last_seen)}`);
  const sum = (x, k) => x.apps.reduce((s, a) => s + a[k], 0);
  const appsCpu = sum(m, 'cpu'), appsMem = sum(m, 'mem'), appsDisk = sum(m, 'disk');
  const lg = (a, b) => h('div', { class: 'legend' }, h('span', {}, h('i', { class: 'apps' }), `apps ${a}`), h('span', {}, h('i', { class: 'sys' }), `system ${b}`));
  return h('div', { class: 'stats' },
    statCard('CPU', h('span', {}, pct(m.cpu), h('small', {}, `load ${m.load.map((l) => l.toFixed(2)).join(' ')}`)), `${host.info.cpus} vCPU`,
      [spark(hist.map((x) => x.cpu), 100, hist.map((x) => sum(x, 'cpu'))), lg(pct(appsCpu), pct(m.cpu - appsCpu))]),
    statCard('Memory', h('span', {}, bytes(m.mem_used), h('small', {}, `of ${bytes(m.mem_total)}`)), m.swap_total ? `swap ${bytes(m.swap_used)}` : null,
      [spark(hist.map((x) => x.mem_used), m.mem_total, hist.map((x) => sum(x, 'mem'))), lg(bytes(appsMem), bytes(m.mem_used - appsMem))]),
    statCard('Disk', h('span', {}, bytes(m.disk_used), h('small', {}, `of ${bytes(m.disk_total)}`)), pct((m.disk_used / m.disk_total) * 100),
      [bar([['apps', (appsDisk / m.disk_total) * 100], ['sys', ((m.disk_used - appsDisk) / m.disk_total) * 100]]), lg(bytes(appsDisk), bytes(m.disk_used - appsDisk))]),
    statCard('Network', h('span', {}, `${bytes(m.net_rx)}/s`, h('small', {}, `↑ ${bytes(m.net_tx)}/s`)), `up ${uptime(m.uptime)}`,
      spark(hist.map((x) => x.net_rx), Math.max(1, ...hist.map((x) => Math.max(x.net_rx, x.net_tx))), hist.map((x) => x.net_tx))));
}

function statusDot(app, metrics) {
  const st = app.last_status;
  if (st === 'running') return h('span', { class: 'dot run', title: 'deploying' });
  if (!metrics) return h('span', { class: 'dot', title: 'not deployed' });
  return h('span', { class: 'dot ' + (metrics.running ? 'ok' : 'err'), title: metrics.running ? 'running' : 'stopped' });
}

function appsTable(host) {
  const byName = Object.fromEntries((host.metrics?.apps || []).map((a) => [a.name, a]));
  if (!host.apps.length) return h('div', { class: 'empty' }, h('h2', {}, 'No apps on this host'), h('p', {}, 'Link a Git repository to deploy a Node.js app.'));
  return h('div', { class: 'table-wrap' }, h('table', {},
    h('thead', {}, h('tr', {}, ['App', 'Branch', 'CPU', 'Memory', 'Disk', 'Last deploy'].map((t, i) => h('th', { class: i > 1 && i < 5 ? 'num' : '' }, t)))),
    h('tbody', {}, host.apps.map((a) => {
      const m = byName[a.spec.name];
      const lim = a.spec.memory_mb ? ` / ${a.spec.memory_mb} MB` : '';
      return h('tr', { class: 'link', onclick: () => (location.hash = `#/apps/${a.id}`) },
        h('td', {}, h('div', { class: 'row' }, statusDot(a, m), h('a', { href: `#/apps/${a.id}` }, h('strong', {}, a.spec.name)), a.spec.port && h('span', { class: 'dim mono small' }, `:${a.spec.port}`))),
        h('td', { class: 'mono small muted' }, `${a.spec.branch} @ ${short(a.last_sha)}`),
        h('td', { class: 'num mono' }, m ? pct(m.cpu) : '—'),
        h('td', { class: 'num mono' }, m ? bytes(m.mem) + lim : '—'),
        h('td', { class: 'num mono' }, m ? bytes(m.disk) : '—'),
        h('td', { class: 'muted' }, a.last_status ? h('span', { class: a.last_status === 'failed' ? 'badge red' : '' }, `${a.last_status} · ${ago(a.last_deploy)}`) : 'never'));
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
  const el = h('div', { class: 'form' },
    isNew && field('Name', f.name, 'Lowercase, used for the systemd unit and folders.'),
    field('Git repository', f.repo, 'Any provider. Private repos: generate a deploy key after creating the app.'),
    field('Branch', f.branch), field('Port', f.port, 'Exported as PORT.'),
    field('Install command', f.install_cmd), field('Build command', f.build_cmd), field('Start command', f.start_cmd),
    field('CPU limit (%)', f.cpu_percent, '100 = one full core.'), field('Memory limit (MB)', f.memory_mb, 'Hard limit, the app is restarted if it exceeds it.'),
    field('Cache paths', f.cache_paths, 'Relative to the app, one per line. Wiped by “Clear cache”.'),
    h('label', { class: 'check full' }, f.auto_deploy, h('span', {}, 'Deploy automatically on every new commit')));
  return { el, read };
}

function newAppDialog(hostId) {
  const form = specForm({}, true);
  const btn = h('button', { class: 'btn primary', type: 'button', onclick: () => act(btn, async () => {
    const r = await api(`/api/hosts/${hostId}/apps`, { method: 'POST', body: form.read() });
    document.querySelector('dialog')?.close();
    location.hash = `#/apps/${r.id}/settings`;
  }, 'App created. Add environment variables or a deploy key, then deploy.') }, 'Create app');
  modal('New app', form.el, btn);
}

async function hostPage(id, tab = 'apps') {
  let host = await api(`/api/hosts/${id}`);
  const live = h('div', {});
  const head = h('div', {});
  const body = h('div', {});
  const drawHead = () => {
    const i = host.info;
    const canUpdate = host.online && host.latest_agent && host.latest_agent !== i.version;
    const upd = h('button', { class: 'btn', onclick: () => act(upd, () => api(`/api/hosts/${id}/update`, { method: 'POST' }), (r) => r.output) }, `Update agent to ${host.latest_agent}`);
    head.replaceChildren(h('div', { class: 'page-head' },
      h('div', {}, h('h1', {}, host.name), h('div', { class: 'sub' },
        h('span', { class: 'row' }, h('span', { class: 'dot ' + (host.online ? 'ok' : 'err') }), host.online ? 'Online' : `Offline · ${ago(host.last_seen)}`),
        [i.hostname, i.os, i.kernel && `kernel ${i.kernel}`, i.arch].filter(Boolean).map((x) => h('span', {}, x)),
        i.firewall && h('span', { class: 'badge' }, `firewall: ${i.firewall}`), versionBadge(i, host.latest_agent))),
      h('div', { class: 'row' }, canUpdate && upd, tab === 'apps' && h('button', { class: 'btn primary', onclick: () => newAppDialog(id) }, 'New app'))));
  };
  const drawLive = () => {
    drawHead();
    live.replaceChildren(hostStats(host), tab === 'apps' ? appsTable(host) : '');
  };

  if (tab === 'keys') {
    const name = h('input', { placeholder: 'laptop' });
    const key = h('textarea', { class: 'mono', placeholder: 'ssh-ed25519 AAAA… you@laptop', rows: 3 });
    const add = h('button', { class: 'btn primary', type: 'button', onclick: () => act(add, async () => {
      await api(`/api/hosts/${id}/ssh-keys`, { method: 'POST', body: { name: name.value, pubkey: key.value } });
      render();
    }, 'Key added') }, 'Add key');
    body.append(h('div', { class: 'stack' },
      h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'SSH keys'), h('p', { class: 'muted small' }, `Installed in ~nextpane/.ssh/authorized_keys. Connect with ssh nextpane@${host.info.hostname || 'host'}.`))),
        host.ssh_keys.length ? h('table', {}, h('tbody', {}, host.ssh_keys.map((k) => h('tr', {},
          h('td', {}, h('strong', {}, k.name || 'key')), h('td', { class: 'mono small muted' }, k.pubkey.slice(0, 60) + '…'),
          h('td', { class: 'num' }, h('button', { class: 'btn sm', onclick: async (e) => {
            if (await confirmDialog('Remove key', `Remove “${k.name}” from this host?`, 'Remove')) act(e.target, () => api(`/api/hosts/${id}/ssh-keys/${k.id}`, { method: 'DELETE' }).then(render));
          } }, 'Remove')))))) : h('div', { class: 's-body muted' }, 'No keys yet.')),
      h('div', { class: 'section' }, h('div', { class: 's-head' }, h('h2', {}, 'Add a key')), h('div', { class: 's-body form' }, field('Name', name), h('div', { class: 'full' }, field('Public key', key))), h('div', { class: 's-foot' }, h('span', {}), add))));
  } else if (tab === 'settings') {
    const name = h('input', { value: host.name });
    const save = h('button', { class: 'btn primary', type: 'button', onclick: () => act(save, () => api(`/api/hosts/${id}`, { method: 'PATCH', body: { name: name.value } }).then(render), 'Saved') }, 'Save');
    const reset = h('button', { class: 'btn', type: 'button', onclick: async () => {
      if (await confirmDialog('Allow re-enrollment', 'The next agent that connects with this machine-id and a valid enrollment key takes over this host. Use it after reinstalling the agent.', 'Allow', false))
        act(reset, () => api(`/api/hosts/${id}`, { method: 'PATCH', body: { reset_identity: true } }), 'Re-enrollment allowed');
    } }, 'Allow re-enrollment');
    const del = h('button', { class: 'btn danger', type: 'button', onclick: async () => {
      if (await confirmDialog('Delete host', 'Removes the host and its apps from NextPane. Nothing is deleted on the server itself; uninstall the agent there.', 'Delete host'))
        act(del, async () => { await api(`/api/hosts/${id}`, { method: 'DELETE' }); location.hash = '#/'; });
    } }, 'Delete host');
    body.append(h('div', { class: 'stack' },
      h('div', { class: 'section' }, h('div', { class: 's-head' }, h('h2', {}, 'Display name')), h('div', { class: 's-body' }, name), h('div', { class: 's-foot' }, h('span', { class: 'dim small' }, `Install method: ${host.info.install || '?'} · machine-id identity`), save)),
      h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'Agent identity'), h('p', { class: 'muted small' }, 'Each host proves its identity with a secret issued at enrollment.'))), h('div', { class: 's-foot' }, h('span', {}), reset)),
      h('div', { class: 'section danger' }, h('div', { class: 's-head' }, h('h2', {}, 'Delete host')), h('div', { class: 's-foot' }, h('span', { class: 'small muted' }, 'This cannot be undone.'), del))));
  }

  shell([[host.name, `#/hosts/${id}`]], head, tabs(`#/hosts/${id}`, tab, [['apps', 'Overview'], ['keys', 'SSH keys'], ['settings', 'Settings']]), live, body);
  drawLive();
  every(5000, async () => { try { host = await api(`/api/hosts/${id}`); drawLive(); } catch {} });
}

// ---------- app ----------

async function appPage(id, tab = 'deployments') {
  let app = await api(`/api/apps/${id}`);
  const s = app.spec;
  const head = h('div', {});
  const body = h('div', {});
  const run = (btn, fn, msg) => act(btn, fn, msg).then(() => refresh());

  const refresh = async () => {
    try { app = await api(`/api/apps/${id}`); } catch { return; }
    const m = app.metrics;
    const deploying = app.deployments[0]?.status === 'running';
    const deploy = h('button', { class: 'btn primary', disabled: !app.online || deploying, onclick: () => run(deploy, () => api(`/api/apps/${id}/deploy`, { method: 'POST', body: {} }), 'Deployment started') }, deploying ? 'Deploying…' : 'Deploy');
    const action = (a, label) => { const b = h('button', { class: 'btn', disabled: !app.online, onclick: () => run(b, () => api(`/api/apps/${id}/action`, { method: 'POST', body: { action: a } }), `${label}: done`) }, label); return b; };
    const cache = h('button', { class: 'btn', disabled: !app.online, onclick: async () => {
      if (await confirmDialog('Clear cache', `Delete ${s.cache_paths.join(', ') || 'nothing (no cache paths configured)'} and restart ${s.name}?`, 'Clear cache', false)) run(cache, () => api(`/api/apps/${id}/cache`, { method: 'POST' }), 'Cache cleared');
    } }, 'Clear cache');
    head.replaceChildren(h('div', { class: 'page-head' },
      h('div', {}, h('h1', { class: 'row' }, statusDot({ last_status: app.deployments[0]?.status }, m), s.name), h('div', { class: 'sub' },
        h('span', { class: 'mono' }, `${s.repo.replace(/^https?:\/\/[^@]*@/, 'https://')}`), h('span', { class: 'badge mono' }, `${s.branch} @ ${short(app.last_sha)}`),
        s.port && h('span', { class: 'mono' }, `:${s.port}`), !app.online && h('span', { class: 'badge red' }, 'host offline'))),
      h('div', { class: 'row' }, m?.running ? action('stop', 'Stop') : action('start', 'Start'), action('restart', 'Restart'), cache, deploy)),
    h('div', { class: 'stats' },
      statCard('Status', m ? (m.running ? 'Running' : 'Stopped') : 'Not deployed', s.auto_deploy ? 'auto-deploy on' : 'manual deploys'),
      statCard('CPU', pct(m?.cpu), s.cpu_percent ? `limit ${s.cpu_percent}%` : 'no limit', s.cpu_percent ? bar([['apps', ((m?.cpu || 0) / s.cpu_percent) * 100]]) : null),
      statCard('Memory', bytes(m?.mem), s.memory_mb ? `limit ${s.memory_mb} MB` : 'no limit', s.memory_mb ? bar([[(m?.mem || 0) > s.memory_mb * KB * KB * 0.9 ? 'hot' : 'apps', ((m?.mem || 0) / (s.memory_mb * KB * KB)) * 100]]) : null),
      statCard('Disk', bytes(m?.disk), 'code, deps, builds')));
    if (tab === 'deployments') drawDeployments();
  };

  // deployments
  const depList = h('div', {});
  const openLog = h('div', {});
  const drawDeployments = () => {
    depList.replaceChildren(app.deployments.length ? h('div', { class: 'table-wrap' }, h('table', {},
      h('thead', {}, h('tr', {}, ['Status', 'Commit', 'Trigger', 'Started', 'Duration'].map((t) => h('th', {}, t)))),
      h('tbody', {}, app.deployments.map((d) => h('tr', { class: 'link', onclick: () => showLog(d.id) },
        h('td', {}, h('span', { class: 'row' }, h('span', { class: 'dot ' + ({ success: 'ok', failed: 'err', running: 'run' }[d.status] || '') }), d.status)),
        h('td', { class: 'mono' }, short(d.sha)), h('td', { class: 'muted' }, d.trigger), h('td', { class: 'muted' }, ago(d.started)), h('td', { class: 'mono muted' }, dur(d.started, d.finished) || '…'))))))
      : h('div', { class: 'empty' }, h('h2', {}, 'No deployments yet'), h('p', {}, app.online ? 'Press Deploy to build and start the app.' : 'The host is offline.')));
  };
  let logTimer;
  const showLog = async (depId) => {
    clearTimeout(logTimer);
    const d = await api(`/api/deployments/${depId}`);
    const pre = h('pre', { class: 'log' }, d.log || (d.status === 'running' ? 'Running… the full log appears when the deployment finishes.' : '(empty)'));
    openLog.replaceChildren(h('div', { class: 'section' }, h('div', { class: 's-head' }, h('h2', {}, `Deployment #${d.id} · ${short(d.sha)} · ${d.status}`), copyBtn(() => d.log)), h('div', { class: 's-body' }, pre)));
    pre.scrollTop = pre.scrollHeight;
    if (d.status === 'running') logTimer = setTimeout(() => location.hash.startsWith(`#/apps/${id}`) && showLog(depId), 3000);
  };

  if (tab === 'deployments') {
    const hook = `${location.origin}${app.webhook_path}`;
    body.append(h('div', { class: 'stack' }, depList, openLog,
      h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'Deploy hook'), h('p', { class: 'muted small' }, 'POST this URL from any Git provider (GitHub, GitLab, Gitea, Forgejo…) to deploy instantly. Pushes to other branches are ignored.'))),
        h('div', { class: 's-body' }, code(hook)))));
  } else if (tab === 'logs') {
    const pre = h('pre', { class: 'log' }, 'Loading…');
    const lines = h('select', { 'aria-label': 'Lines' }, [100, 300, 1000, 3000].map((n) => h('option', { value: n, selected: n === 300 }, `${n} lines`)));
    lines.style.width = 'auto';
    const load = async () => {
      try { const r = await api(`/api/apps/${id}/logs?lines=${lines.value}`); pre.textContent = r.output || '(no output yet)'; pre.scrollTop = pre.scrollHeight; } catch (e) { pre.textContent = e.message; }
    };
    lines.onchange = load;
    const follow = h('input', { type: 'checkbox', checked: true });
    body.append(h('div', { class: 'stack' }, h('div', { class: 'row' }, h('span', { class: 'muted grow' }, `journalctl -u nextpane-app-${s.name}`), h('label', { class: 'check small muted' }, follow, 'Auto-refresh'), lines, h('button', { class: 'btn sm', onclick: load }, 'Refresh')), pre));
    load();
    every(5000, () => follow.checked && load());
  } else if (tab === 'env') {
    body.append(await envEditor(id, s.name));
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
    const del = h('button', { class: 'btn danger', type: 'button', onclick: async () => {
      if (!(await confirmDialog('Delete app', `Stops ${s.name}, removes its systemd unit, firewall rules, env file and code from the host.`, 'Delete app'))) return;
      act(del, async () => {
        try { await api(`/api/apps/${id}`, { method: 'DELETE' }); } catch (e) {
          if (!(await confirmDialog('Host unreachable', `${e.message}\n\nDelete it from NextPane anyway? Files stay on the host.`, 'Delete anyway'))) return;
          await api(`/api/apps/${id}?force=true`, { method: 'DELETE' });
        }
        location.hash = `#/hosts/${app.host_id}`;
      });
    } }, 'Delete app');
    body.append(h('div', { class: 'stack' },
      h('div', { class: 'section' }, h('div', { class: 's-head' }, h('h2', {}, 'Build & runtime')), h('div', { class: 's-body' }, form.el), h('div', { class: 's-foot' }, h('span', { class: 'dim small' }, 'Limits and commands apply immediately (the app restarts).'), save)),
      h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'Deploy key'), h('p', { class: 'muted small' }, 'For private repositories: an SSH key generated on the host (the private half never leaves it). Add the public key as a read-only deploy key in your Git provider and use the SSH clone URL.'))),
        h('div', { class: 's-body' }, keyBox), h('div', { class: 's-foot' }, h('span', {}), gen)),
      h('div', { class: 'section danger' }, h('div', { class: 's-head' }, h('h2', {}, 'Delete app')), h('div', { class: 's-foot' }, h('span', { class: 'small muted' }, 'This cannot be undone.'), del))));
  }

  shell([[app.host_name, `#/hosts/${app.host_id}`], [s.name, `#/apps/${id}`]], head,
    tabs(`#/apps/${id}`, tab, [['deployments', 'Deployments'], ['logs', 'Logs'], ['env', 'Environment'], ['firewall', 'Firewall'], ['settings', 'Settings']]), body);
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
      h('button', { class: 'btn ghost sm', type: 'button', onclick: (e) => { val.type = val.type === 'password' ? 'text' : 'password'; e.target.textContent = val.type === 'password' ? 'Show' : 'Hide'; } }, 'Show'),
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
      document.querySelector('dialog')?.close();
    }, 'Exported (recorded in the audit log)') }, 'Export');
    modal('Export environment', h('div', { class: 'stack' },
      h('p', { class: 'muted' }, 'Exports are recorded in the audit log. Prefer an encrypted export: plain files leak easily through downloads folders, backups and chat.'),
      field('Your password', pw), field('Encryption passphrase (recommended)', pass, 'Decrypt with: openssl enc -d -aes-256-cbc -pbkdf2 -iter 600000 -md sha256 -a -A -in FILE')), go);
  } }, 'Export…');
  return h('div', { class: 'section' },
    h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'Environment variables'), h('p', { class: 'muted small' }, `Encrypted at rest on the panel, written to /etc/nextpane/apps/${name}.env (root, 0600) on the host. Available during build and at runtime.`)), h('div', { class: 'row' }, paste, exp)),
    h('div', { class: 's-body' }, rows, h('button', { class: 'btn sm', type: 'button', onclick: () => addRow() }, '+ Add variable')),
    h('div', { class: 's-foot' }, h('label', { class: 'check small muted' }, apply, 'Apply now (restarts the app)'), save));
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

// ---------- settings ----------

async function settingsPage() {
  const [sys, { keys }, { entries }] = await Promise.all([api('/api/system'), api('/api/enroll-keys'), api('/api/audit')]);
  const docker = sys.install === 'docker';
  const upd = h('button', { class: 'btn primary', disabled: !sys.update_available || docker, onclick: async () => {
    if (await confirmDialog('Update NextPane', `Install ${sys.latest}? The panel restarts and will be unavailable for a few seconds. Agents reconnect automatically.`, 'Update', false))
      act(upd, () => api('/api/system/update', { method: 'POST' }), (r) => r.output).then(() => setTimeout(() => location.reload(), 8000));
  } }, sys.update_available ? `Update to ${sys.latest}` : 'Up to date');
  const create = h('button', { class: 'btn', type: 'button', onclick: addHostDialog }, 'New key');
  const cur = h('input', { type: 'password', autocomplete: 'current-password' });
  const nw = h('input', { type: 'password', autocomplete: 'new-password', minlength: 10 });
  const pw = h('button', { class: 'btn primary', type: 'button', onclick: () => act(pw, async () => { await api('/api/password', { method: 'POST', body: { current: cur.value, new: nw.value } }); location.hash = '#/login'; }, 'Password changed, sign in again') }, 'Change password');

  shell([['Settings', '#/settings']], h('div', { class: 'page-head' }, h('h1', {}, 'Settings')), h('div', { class: 'stack' },
    h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'NextPane server'), h('p', { class: 'muted small' }, `Version ${sys.version} · installed as ${sys.install}`))),
      h('div', { class: 's-body muted' },
        !sys.updates_enabled ? 'Updates are disabled: set NEXTPANE_RELEASE_URL in the server configuration.'
          : sys.error ? `Cannot check for updates: ${sys.error}`
            : docker ? `Latest: ${sys.latest}. Running in Docker: update with “docker compose pull && docker compose up -d”.`
              : `Latest release: ${sys.latest}.`),
      h('div', { class: 's-foot' }, h('span', { class: 'dim small' }, 'Agents are updated from each host page.'), upd)),
    h('div', { class: 'section' }, h('div', { class: 's-head' }, h('div', {}, h('h2', {}, 'Enrollment keys'), h('p', { class: 'muted small' }, 'Let new agents register. Revoking a key does not disconnect hosts that already enrolled.')), create),
      keys.length ? h('table', {}, h('tbody', {}, keys.map((k) => h('tr', {}, h('td', {}, h('strong', {}, k.name)), h('td', { class: 'muted' }, `created ${ago(k.created)}`),
        h('td', { class: 'num' }, h('button', { class: 'btn sm', onclick: async (e) => {
          if (await confirmDialog('Revoke key', `Revoke “${k.name}”? New hosts can no longer enroll with it.`, 'Revoke')) act(e.target, () => api(`/api/enroll-keys/${k.id}`, { method: 'DELETE' }).then(render));
        } }, 'Revoke')))))) : h('div', { class: 's-body muted' }, 'No active keys.')),
    h('div', { class: 'section' }, h('div', { class: 's-head' }, h('h2', {}, 'Password')), h('div', { class: 's-body form' }, field('Current password', cur), field('New password', nw, 'At least 10 characters. Signs out every session.')), h('div', { class: 's-foot' }, h('span', {}), pw)),
    h('div', { class: 'section' }, h('div', { class: 's-head' }, h('h2', {}, 'Audit log')),
      entries.length ? h('div', { class: 'table-wrap' }, h('table', {}, h('tbody', {}, entries.map((e) => h('tr', {}, h('td', { class: 'muted small' }, new Date(e.ts * 1000).toLocaleString()), h('td', { class: 'mono small' }, e.action), h('td', { class: 'small' }, e.detail)))))) : h('div', { class: 's-body muted' }, 'Nothing yet.'))));
}

// ---------- router ----------

const routes = [
  [/^#\/hosts\/(\d+)(?:\/(\w+))?$/, hostPage],
  [/^#\/apps\/(\d+)(?:\/(\w+))?$/, appPage],
  [/^#\/settings$/, settingsPage],
  [/^(#\/?)?$/, overview],
];

async function render() {
  timers.forEach(clearInterval);
  timers = [];
  document.querySelectorAll('dialog').forEach((d) => d.close());
  const hash = location.hash;
  if (hash === '#/login' || hash === '#/setup') {
    const me = await api('/api/me');
    if (me.user) { location.hash = '#/'; return; }
    return authPage(me.needs_setup);
  }
  for (const [re, view] of routes) {
    const m = hash.match(re);
    if (!m) continue;
    try {
      await view(...m.slice(1).filter((x) => x !== undefined && !x.startsWith('#')));
    } catch (e) {
      if (!e.auth) shell([], h('div', { class: 'empty' }, h('h2', {}, 'Something went wrong'), h('p', {}, e.message), h('a', { class: 'btn', href: '#/' }, 'Back to hosts')));
    }
    return;
  }
  shell([], h('div', { class: 'empty' }, h('h2', {}, 'Page not found'), h('a', { class: 'btn', href: '#/' }, 'Back to hosts')));
}

window.addEventListener('hashchange', render);
(async () => {
  const me = await api('/api/me').catch(() => ({}));
  const target = me.needs_setup ? '#/setup' : '#/login';
  if (!me.user && location.hash !== target) location.hash = target; // hashchange renders
  else render();
})();
