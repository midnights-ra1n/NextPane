// NextPane demo: real server + simulated agents speaking the real protocol.
//   node dev/demo.mjs            then open http://localhost:3000  (admin / nextpane-demo)
// Fresh state on every run (dev/.demo-data). Ctrl+C stops everything.
// Simulated agents deploy nothing: they fake build logs, metrics, journal lines and self-updates.

import { spawn, spawnSync } from 'node:child_process';
import { rmSync, mkdirSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const PORT = Number(process.env.PORT || 3000);
const BASE = `http://127.0.0.1:${PORT}`;
const DATA = fileURLToPath(new URL('./.demo-data', import.meta.url));
const USER = 'admin', PASS = 'nextpane-demo';
const GiB = 1024 ** 3, MiB = 1024 ** 2;

const HOSTS = [
  { id: 1, hostname: 'web-paris-1', os: 'Debian GNU/Linux 12 (bookworm)', kernel: '6.1.0-28-amd64', arch: 'x86_64', cpus: 4, mem: 8 * GiB, disk: 160e9, firewall: 'ufw', install: 'deb', version: '0.1.0', sys: 6 },
  { id: 2, hostname: 'web-paris-2', os: 'Ubuntu 24.04.1 LTS', kernel: '6.8.0-49-generic', arch: 'x86_64', cpus: 2, mem: 4 * GiB, disk: 80e9, firewall: 'nftables', install: 'deb', version: '0.0.9', sys: 9 },
  { id: 3, hostname: 'api-fra-1', os: 'AlmaLinux 9.5 (Teal Serval)', kernel: '5.14.0-503.el9.aarch64', arch: 'aarch64', cpus: 8, mem: 16 * GiB, disk: 320e9, firewall: 'firewalld', install: 'rpm', version: '0.1.0', sys: 4 },
  { id: 4, hostname: 'staging-1', os: 'Rocky Linux 9.4 (Blue Onyx)', kernel: '5.14.0-427.el9.x86_64', arch: 'x86_64', cpus: 2, mem: 2 * GiB, disk: 40e9, firewall: 'iptables', install: 'rpm', version: '0.1.0', sys: 3, goesOffline: true },
];

const APPS = {
  'web-paris-1': [
    { name: 'site-vitrine', repo: 'git@github.com:acme/site-vitrine.git', branch: 'main', port: 3000, memory_mb: 512, cpu_percent: 100, cache_paths: ['.next/cache'], auto_deploy: true, firewall: [] },
    { name: 'blog', repo: 'https://github.com/acme/blog.git', branch: 'main', port: 3001, memory_mb: 256, cache_paths: ['.next/cache', 'node_modules/.cache'], auto_deploy: true, firewall: [] },
  ],
  'web-paris-2': [
    { name: 'docs', repo: 'https://gitea.acme.dev/acme/docs.git', branch: 'main', port: 3000, cache_paths: ['.next/cache'], auto_deploy: true, firewall: [{ port: 3000, proto: 'tcp', source: null }] },
    { name: 'discord-bot', repo: 'https://github.com/acme/discord-bot.git', branch: 'main', start_cmd: 'node index.js', memory_mb: 256, cache_paths: [], auto_deploy: true, firewall: [] },
  ],
  'api-fra-1': [
    { name: 'api', repo: 'git@gitlab.com:acme/api.git', branch: 'production', port: 8080, memory_mb: 1024, cpu_percent: 200, cache_paths: [], auto_deploy: true, firewall: [{ port: 8080, proto: 'tcp', source: '10.0.0.0/8' }, { port: 8080, proto: 'tcp', source: '2001:db8::/32' }] },
    { name: 'worker', repo: 'git@gitlab.com:acme/api.git', branch: 'production', start_cmd: 'node dist/worker.js', memory_mb: 512, cache_paths: [], auto_deploy: false, firewall: [] },
    { name: 'legacy-admin', repo: 'git@forgejo.acme.dev:acme/legacy-admin.git', branch: 'main', port: 4000, cache_paths: [], auto_deploy: false, firewall: [] },
  ],
  'staging-1': [
    { name: 'preview', repo: 'git@github.com:acme/site-vitrine.git', branch: 'develop', port: 3000, cache_paths: ['.next/cache'], auto_deploy: true, firewall: [] },
  ],
};
const ENV = {
  api: [{ key: 'DATABASE_URL', value: 'postgres://api:s3cr3t@10.0.0.5:5432/api' }, { key: 'REDIS_URL', value: 'redis://10.0.0.6:6379' }, { key: 'JWT_SECRET', value: randomBytes(24).toString('hex') }],
  // Fake token: the Discord tab shows how NextPane reports a rejected token.
  'discord-bot': [{ key: 'DISCORD_TOKEN', value: 'MTEzNDU2Nzg5MDEyMzQ1Njc4.demo.' + randomBytes(16).toString('hex') }],
  'site-vitrine': [{ key: 'NEXT_PUBLIC_SITE_URL', value: 'https://acme.example' }, { key: 'REVALIDATE_TOKEN', value: randomBytes(16).toString('hex') }],
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const rand = (a, b) => a + Math.random() * (b - a);
const walk = (v, lo, hi, step) => Math.min(hi, Math.max(lo, v + rand(-step, step)));
const sha = () => randomBytes(20).toString('hex');
const log = (...a) => console.log('\x1b[2m[demo]\x1b[0m', ...a);

// ---------- server ----------

let server;
function startServer() {
  log('building nextpane-server (release)…');
  const b = spawnSync('cargo', ['build', '--release', '-q', '-p', 'nextpane-server'], { stdio: 'inherit' });
  if (b.status !== 0) process.exit(1);
  rmSync(DATA, { recursive: true, force: true });
  mkdirSync(DATA, { recursive: true });
  server = spawn('./target/release/nextpane-server', [], {
    env: { ...process.env, NEXTPANE_DATA: DATA, NEXTPANE_LISTEN: `127.0.0.1:${PORT}`, NEXTPANE_METRICS_INTERVAL: '2', NEXTPANE_POLL_INTERVAL: '20', NEXTPANE_RELEASE_URL: 'https://github.com/midnights-ra1n/NextPane/releases' },
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  return new Promise((resolve, reject) => {
    let out = '';
    server.stdout.on('data', (d) => {
      out += d;
      const m = out.match(/setup code: ([a-f0-9]+)/);
      if (m && out.includes('listening')) resolve(m[1]);
    });
    server.on('exit', (c) => reject(new Error(`server exited (${c}): is port ${PORT} already in use?`)));
  });
}

let cookie = '';
async function api(path, body, method) {
  const r = await fetch(BASE + path, { method: method || (body ? 'POST' : 'GET'), headers: { 'content-type': 'application/json', cookie }, body: body && JSON.stringify(body) });
  const sc = r.headers.get('set-cookie');
  if (sc) cookie = sc.split(';')[0];
  const data = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(`${path}: ${data.error || r.status}`);
  return data;
}

// ---------- simulated agent ----------

class SimHost {
  constructor(def, token) {
    Object.assign(this, def);
    this.token = token;
    this.machineId = `5eed${String(def.id).padStart(28, '0')}`;
    this.apps = new Map(); // name -> state
    this.cpuSys = def.sys; this.memSys = def.mem * rand(0.18, 0.3); this.diskSys = def.disk * rand(0.15, 0.3);
    this.rx = 0; this.tx = 0; this.uptime = Math.floor(rand(3, 90) * 86400);
    this.metricsEvery = 10; this.pollEvery = 60;
  }

  connect() {
    const headers = { 'x-machine-id': this.machineId, authorization: `Bearer ${this.token}` };
    if (this.secret) headers['x-host-secret'] = this.secret;
    const ws = (this.ws = new WebSocket(`ws://127.0.0.1:${PORT}/agent/ws`, { headers }));
    ws.onopen = () => {
      this.send({ t: 'hello', version: this.version, hostname: this.hostname, os: this.os, kernel: this.kernel, arch: this.arch, cpus: this.cpus, mem_total: this.mem, firewall: this.firewall, install: this.install });
      this.schedule();
    };
    ws.onmessage = (e) => this.onMessage(JSON.parse(e.data));
    ws.onclose = () => {
      this.stop();
      if (!this.offline) setTimeout(() => this.connect(), 3000);
    };
    ws.onerror = () => {};
  }

  send(m) { if (this.ws?.readyState === 1) this.ws.send(JSON.stringify(m)); }
  stop() { clearInterval(this.mt); clearInterval(this.pt); }
  schedule() {
    this.stop();
    this.mt = setInterval(() => this.metrics(), this.metricsEvery * 1000);
    this.pt = setInterval(() => this.poll(), this.pollEvery * 1000);
  }

  app(spec) {
    if (!this.apps.has(spec.name)) this.apps.set(spec.name, { spec, running: false, deployed: null, remote: sha(), cpu: rand(1, 15), mem: rand(80, 300) * MiB, disk: 0 });
    const a = this.apps.get(spec.name);
    a.spec = spec;
    return a;
  }

  onMessage(m) {
    if (m.t === 'welcome') this.secret = m.host_secret;
    if (m.t === 'sync') {
      this.metricsEvery = m.metrics_interval; this.pollEvery = m.poll_interval;
      const names = new Set(m.apps.map((s) => s.name));
      for (const n of this.apps.keys()) if (!names.has(n)) this.apps.delete(n);
      m.apps.forEach((s) => this.app(s));
      this.schedule();
    }
    if (m.t === 'cmd') this.command(m.id, m.cmd).catch((e) => this.send({ t: 'result', id: m.id, ok: false, output: String(e) }));
  }

  async command(id, c) {
    const reply = (ok, output, data = null) => this.send({ t: 'result', id, ok, output, data });
    const a = c.app && typeof c.app === 'object' ? this.app(c.app) : this.apps.get(c.app);
    switch (c.kind) {
      case 'deploy': return this.deploy(a, c, reply);
      case 'configure': return reply(true, `configuration applied\nfirewall (${this.firewall}): ${a.spec.firewall.length} rule(s) applied\n`);
      case 'action':
        await sleep(600);
        if (!a?.deployed) return reply(false, `Failed to ${c.action} nextpane-app-${c.app}.service: Unit not found.`);
        a.running = c.action !== 'stop';
        return reply(true, '');
      case 'remove':
        await sleep(800);
        this.apps.delete(c.app);
        return reply(true, `firewall (${this.firewall}): 0 rule(s) applied\n${c.app} removed\n`);
      case 'clear_cache':
        await sleep(700);
        return reply(true, c.paths.map((p) => `cleared ${p}\n`).join(''));
      case 'logs': return reply(true, this.journal(c.app, Math.min(c.lines, 400)));
      case 'deploy_key': return reply(true, '', `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI${randomBytes(24).toString('base64').replace(/[+/=]/g, 'x')} nextpane@${c.app}`);
      case 'ssh_keys': return reply(true, `${c.keys.length} authorized key(s) installed\n`);
      case 'self_update':
        reply(true, `updating to ${c.version} (${this.install}), the agent will reconnect shortly\n`);
        await sleep(2500);
        this.version = c.version;
        this.ws.close();
        return;
      default: return reply(false, `unknown command ${c.kind}`);
    }
  }

  async deploy(a, c, reply) {
    const s = a.spec, target = c.sha || a.remote;
    const pm = s.name === 'blog' ? 'pnpm' : 'npm';
    let out = `$ git fetch ${s.repo} ${s.branch}\nFrom ${s.repo.replace(/\.git$/, '')}\n * branch            ${s.branch} -> FETCH_HEAD\n$ checkout ${target}\n\n`;
    await sleep(rand(1500, 2500));
    out += `$ ${pm === 'pnpm' ? 'pnpm install --frozen-lockfile' : 'npm ci'}\n`;
    out += pm === 'pnpm' ? `Lockfile is up to date, resolution step is skipped\nPackages: +${Math.floor(rand(300, 900))}\nDone in ${rand(4, 12).toFixed(1)}s\n` : `added ${Math.floor(rand(300, 900))} packages, and audited ${Math.floor(rand(900, 1000))} packages in ${Math.floor(rand(5, 20))}s\n\nfound 0 vulnerabilities\n`;
    await sleep(rand(2000, 4000));
    out += `\n$ ${pm} run build\n\n> ${s.name}@1.${Math.floor(rand(0, 9))}.0 build\n`;
    if (s.name === 'legacy-admin') {
      await sleep(1500);
      out += `> tsc -p . && next build\n\nsrc/pages/users.tsx(42,7): error TS2322: Type 'string | undefined' is not assignable to type 'string'.\n\nerror: systemd-run failed (exit status: 2)`;
      return reply(false, out);
    }
    out += s.name === 'api' || s.name === 'worker' ? '> tsc -p .\n' : `> next build\n\n   ▲ Next.js 15.1.0\n\n   Creating an optimized production build ...\n ✓ Compiled successfully\n ✓ Generating static pages (${Math.floor(rand(8, 40))}/${Math.floor(rand(40, 60))})\n\nRoute (app)                              Size     First Load JS\n┌ ○ /                                    5.2 kB          110 kB\n└ ○ /_not-found                          979 B           106 kB\n`;
    await sleep(rand(2500, 5000));
    a.deployed = target; a.remote = target; a.running = true;
    a.disk = rand(150, 700) * MiB;
    out += `\nstarted ${s.name} at ${target.slice(0, 12)}\nfirewall (${this.firewall}): ${s.firewall.length} rule(s) applied\n`;
    reply(true, out, target);
  }

  journal(name, n) {
    const a = this.apps.get(name);
    if (!a?.deployed) return '-- No entries --\n';
    const paths = ['/', '/api/health', '/blog', '/products', '/api/users', '/_next/static/chunks/main.js', '/login'];
    const lines = [];
    const now = Date.now();
    if (name === 'discord-bot') {
      const t = (i) => new Date(now - i * 60000).toISOString().replace(/\.\d+Z$/, '+0000');
      const p = `${this.hostname} node[${2000 + this.id}]:`;
      return [`${t(9)} ${p} Loaded 6 slash commands`, `${t(9)} ${p} Ready! Logged in as AcmeBot#4821`, `${t(4)} ${p} DiscordAPIError[50013]: Missing Permissions`,
        `${t(2)} ${p} Error [TokenInvalid]: An invalid token was provided.`, `${t(1)} ${p} Error [TokenInvalid]: An invalid token was provided.`].join('\n') + '\n';
    }
    for (let i = n; i > 0; i--) {
      const t = new Date(now - i * rand(800, 4000)).toISOString().replace(/\.\d+Z$/, '+0000');
      const st = Math.random() < 0.04 ? 500 : Math.random() < 0.1 ? 404 : 200;
      lines.push(`${t} ${this.hostname} ${a.spec.start_cmd ? 'node' : 'npm'}[${1000 + this.id * 37}]: ${st === 500 ? `Error: connect ETIMEDOUT 10.0.0.5:5432` : `${['GET', 'GET', 'GET', 'POST'][i % 4]} ${paths[i % paths.length]} ${st} ${Math.floor(rand(1, 90))}ms`}`);
    }
    return lines.join('\n') + '\n';
  }

  metrics() {
    this.cpuSys = walk(this.cpuSys, 1, 25, 3);
    this.memSys = walk(this.memSys, this.mem * 0.12, this.mem * 0.4, this.mem * 0.01);
    this.rx = walk(this.rx || 2e5, 1e4, 8e6, 4e5); this.tx = walk(this.tx || 8e5, 2e4, 2e7, 9e5);
    this.uptime += this.metricsEvery;
    const apps = [...this.apps.values()].map((a) => {
      if (a.running) {
        const capCpu = a.spec.cpu_percent || 100 * this.cpus;
        a.cpu = walk(a.cpu, 0.5, Math.min(capCpu, 60), 6);
        a.mem = walk(a.mem, 60 * MiB, (a.spec.memory_mb || 900) * MiB * 0.95, 12 * MiB);
      }
      const m = { name: a.spec.name, running: a.running, cpu: a.running ? a.cpu / this.cpus : 0, mem: a.running ? Math.round(a.mem) : 0, disk: Math.round(a.disk) };
      if (a.spec.name === 'discord-bot' && a.deployed) m.discord_js = '14.16.3';
      return m;
    });
    const appCpu = apps.reduce((s, x) => s + x.cpu, 0), appMem = apps.reduce((s, x) => s + x.mem, 0), appDisk = apps.reduce((s, x) => s + x.disk, 0);
    const cpu = Math.min(100, this.cpuSys + appCpu);
    this.send({
      t: 'metrics', ts: Math.floor(Date.now() / 1000), cpu, load: [cpu / 100 * this.cpus, cpu / 100 * this.cpus * 0.9, cpu / 100 * this.cpus * 0.8].map((x) => +x.toFixed(2)),
      mem_used: Math.round(Math.min(this.mem * 0.97, this.memSys + appMem)), mem_total: this.mem, swap_used: this.mem > 4 * GiB ? 0 : Math.round(rand(50, 200) * MiB), swap_total: this.mem > 4 * GiB ? 0 : 2 * GiB,
      disk_used: Math.round(this.diskSys + appDisk), disk_total: this.disk, net_rx: Math.round(this.rx), net_tx: Math.round(this.tx), uptime: this.uptime, apps,
    });
  }

  poll() {
    for (const a of this.apps.values()) {
      if (!a.spec.auto_deploy || !a.deployed) continue;
      if (Math.random() < 0.08) a.remote = sha(); // someone pushed a commit
      this.send({ t: 'head', app: a.spec.name, sha: a.remote });
    }
  }
}

// ---------- main ----------

const code = await startServer();
const cleanup = () => { server?.kill(); process.exit(0); };
process.on('SIGINT', cleanup);
process.on('SIGTERM', cleanup);

await api('/api/setup', { username: USER, password: PASS, code });
const { key } = await api('/api/enroll-keys', { name: 'demo' });
log('admin account and enrollment key created');

const sims = HOSTS.map((d) => new SimHost(d, key));
sims.forEach((s) => s.connect());
while ((await api('/api/hosts')).hosts.filter((h) => h.online).length < HOSTS.length) await sleep(300);
log(`${HOSTS.length} simulated hosts connected`);

const { hosts } = await api('/api/hosts');
const appIds = [];
for (const h of hosts) {
  for (const spec of APPS[h.info.hostname] || []) {
    const { id } = await api(`/api/hosts/${h.id}/apps`, spec);
    if (ENV[spec.name]) await api(`/api/apps/${id}/env`, { vars: ENV[spec.name], apply: false }, 'PUT');
    appIds.push([id, spec.name, h.info.hostname]);
  }
  await api(`/api/hosts/${h.id}/ssh-keys`, { name: 'ops-laptop', pubkey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemoKeyForNextPaneSimulation ops@laptop' });
}
log(`${appIds.length} apps created`);

for (const [id, name, host] of appIds) {
  if (host === 'staging-1') continue;
  await api(`/api/apps/${id}/deploy`, {}).catch((e) => log(`deploy ${name}: ${e.message}`));
}
log('deployments started (they take ~10 s each, legacy-admin fails on purpose)');

// staging-1 deploys its preview, then drops off the network to show an offline host.
const staging = sims.find((s) => s.goesOffline);
const preview = appIds.find(([, , h]) => h === 'staging-1');
if (preview) await api(`/api/apps/${preview[0]}/deploy`, {}).catch(() => {});
setTimeout(() => { staging.offline = true; staging.ws.close(); log('staging-1 went offline'); }, 25000);

console.log(`
  \x1b[1mNextPane demo is running\x1b[0m
    URL       http://localhost:${PORT}
    Login     ${USER} / ${PASS}

  web-paris-2 runs agent 0.0.9: try "Update agent" on its page.
  Auto-deploy: simulated pushes trigger deployments every few minutes.
  Ctrl+C to stop.
`);
