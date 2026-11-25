# Apps & deployments

An **app** is a Node.js project from a Git repository, deployed on one host and run as a systemd
service. Create one from a host page: **New app**.

## Settings

| Field | Default | Notes |
|---|---|---|
| Name | — | 1–24 chars, `a-z 0-9 -`. Used for the unit (`nextpane-app-<name>`) and folders. Cannot change later. |
| Git repository | — | Any URL `git` understands: `https://…`, `git@host:org/repo.git`, `ssh://…`. |
| Branch | `main` | Deployed branch, also used to filter webhooks. |
| Port | `3000` | Exported to the app as `PORT`. |
| Install command | auto | See *Build pipeline*. |
| Build command | auto | See *Build pipeline*. |
| Start command | `<pm> start` | Run with `/bin/sh -c`, so `node server.js`, `npx next start -p $PORT`… work. |
| CPU limit | none | systemd `CPUQuota`, in % of one core (`150` = 1.5 cores). |
| Memory limit | none | systemd `MemoryMax`, in MB. Exceeding it gets the app OOM-killed and restarted. |
| Cache paths | `.next/cache` | Paths relative to the release, wiped by **Clear cache**. |
| Auto-deploy | on | Deploy every new commit of the branch. |

Saving the settings rewrites the unit and restarts the app if it is running.

## Build pipeline

A deployment runs on the host, step by step, and its full log is kept in the panel:

1. **Fetch**: `git fetch` into a bare mirror (`apps/<app>/repo.git`), as the `nextpane` user.
2. **Checkout** the commit into a new release folder `apps/<app>/releases/<time>-<sha>`.
3. **Install** dependencies. Auto-detected from the lockfile:

   | Lockfile | Install | Build | Start |
   |---|---|---|---|
   | `pnpm-lock.yaml` | `pnpm install --frozen-lockfile` | `pnpm run build` | `pnpm start` |
   | `yarn.lock` | `yarn install --frozen-lockfile` | `yarn run build` | `yarn start` |
   | `bun.lock` / `bun.lockb` | `bun install --frozen-lockfile` | `bun run build` | `bun start` |
   | `package-lock.json` | `npm ci` | `npm run build` | `npm start` |
   | none | `npm install` | `npm run build` | `npm start` |

   The build step only runs when `package.json` has a `build` script (or a build command is set).
4. **Build**. Install and build run in a transient systemd unit (`nextpane-build-<app>`) as
   `nextpane`, **with the app's environment variables and resource limits**.
5. **Switch**: the `current` symlink atomically points to the new release, the unit is (re)written and
   the app restarted.
6. **Cleanup**: only the 3 newest releases are kept. Firewall rules are re-applied.

If any step fails, the previous release keeps running untouched.

The running service is `nextpane-app-<app>.service`: `User=nextpane`, `NODE_ENV=production`,
`Restart=on-failure`, `NoNewPrivileges`, `ProtectSystem=full`, `PrivateTmp`.

## Private repositories

Use an SSH **deploy key**, generated on the host (the private key never leaves it):

1. *App › Settings › Deploy key › Generate deploy key*.
2. Copy the public key into your provider as a **read-only deploy key**:
   GitHub *Settings › Deploy keys*, GitLab *Settings › Repository › Deploy keys*,
   Gitea/Forgejo *Settings › Deploy keys*.
3. Use the SSH clone URL (`git@github.com:org/repo.git`).

The first connection accepts the provider's host key (`StrictHostKeyChecking=accept-new`) and pins it in
`/var/lib/nextpane/.ssh/known_hosts`. Pre-fill that file if you want to pin it yourself.

> HTTPS with a token in the URL (`https://user:token@host/repo.git`) also works, but the token is
> then stored in the app settings in clear text. Prefer deploy keys.

## Auto-deploy

Two complementary mechanisms, both provider-agnostic:

- **Polling** (always on for auto-deploy apps): the agent runs `git ls-remote` every
  `NEXTPANE_POLL_INTERVAL` seconds (60 by default) and the panel deploys when the head moved. Works
  behind NAT and with any Git server. A commit that failed is not retried automatically.
- **Webhook** (instant): *App › Deployments › Deploy hook* gives a secret URL. Add it as a push webhook:

  | Provider | Where | Content type |
  |---|---|---|
  | GitHub | *Settings › Webhooks › Add webhook* | `application/json` |
  | GitLab | *Settings › Webhooks*, trigger *Push events* | — |
  | Gitea / Forgejo | *Settings › Webhooks › Add webhook › Gitea/Forgejo* | `POST`, JSON |
  | Anything else | `curl -X POST https://panel/hooks/<id>/<secret>` | — |

  When the payload contains a `ref` (all the providers above), pushes to other branches are ignored.
  The secret is in the URL, so no provider-specific signature is needed.

Manual deploys: **Deploy** button, or `POST /api/apps/<id>/deploy` with `{"sha": "<commit>"}` for a
specific commit (e.g. a rollback).

## Environment variables

*App › Environment*.

- Stored **encrypted** in the panel database (AES-256-GCM, key in `secret.key`).
- Pushed to the host over the agent's TLS WebSocket and written to `/etc/nextpane/apps/<app>.env`
  (root, 0600): systemd reads it, the `nextpane` user cannot.
- Available to the build (e.g. `NEXT_PUBLIC_*`) and at runtime. `PORT` and `NODE_ENV` are set by
  NextPane but your variables win.
- *Paste .env* imports a whole file. Values are masked until you click *Show*.
- Values cannot contain line breaks (systemd limitation): base64-encode multi-line secrets such as PEM keys.
- *Save* with *Apply now* restarts the app; otherwise changes apply at the next deploy.

### Export

Exporting secrets is where leaks happen, so it is guarded:

- you must type your **password** again,
- every export is written to the **audit log** (encrypted or plaintext),
- you are encouraged to set a **passphrase**: the file is then encrypted with AES-256-CBC + PBKDF2
  (600 000 iterations) in the standard OpenSSL format, and can be decrypted anywhere without NextPane:

```sh
openssl enc -d -aes-256-cbc -pbkdf2 -iter 600000 -md sha256 -a -A -in my-app.env.enc > my-app.env
```

## Cache

**Clear cache** deletes every *cache path* of the current release (default `.next/cache`, which holds
Next.js' image, fetch and ISR caches) as the `nextpane` user, then restarts the app. Add other paths
in the settings (`node_modules/.cache`, `.cache`, `public/generated`…). Paths are relative and cannot
contain `..`.

## Logs and actions

- **Logs**: `journalctl` of the app (100 to 3 000 lines, auto-refresh).
- **Start / Stop / Restart**: plain `systemctl` actions.
- **Deployments**: history with status, commit, trigger, duration and full build log.

## Deleting an app

*Settings › Delete app* stops and disables the unit, removes it, closes its firewall rules and deletes
its env file and folder on the host. If the host is offline you can still remove it from the panel;
files then stay on the host.

## Discord bots (discord.js)

A Discord bot is an app without a port. In **New app**, pick the type *Discord bot*: the port and
cache paths are cleared and the start command defaults to `node index.js`.

1. Put the bot token in the **Environment** tab as `DISCORD_TOKEN` (`DISCORD_BOT_TOKEN`, `BOT_TOKEN`
   and `TOKEN` are also read, in that order). It is encrypted at rest like every variable.
2. Deploy. After the deployment the agent finds `node_modules/discord.js` in the release, the app gets
   a *Discord bot* badge and a **Discord** tab.

The Discord tab shows:

- the bot identity, its status (process running, token accepted by Discord, no fatal error in the
  logs) and the discord.js version;
- **diagnostics**: Discord API answers plus known discord.js errors found in the last 300 log lines
  (invalid token, disallowed intents, missing module, missing permissions or access, late interaction
  replies, DNS problems), each with the fix;
- the privileged intents enabled in the Developer Portal;
- an **invite link** builder (no permissions, common permissions or administrator);
- the **servers** the bot is in, with a *Leave* action;
- the **global slash commands**, with a *Delete* action for stale ones. Guild-scoped commands are not
  listed.

The panel calls `discord.com` itself with the token, so the token never reaches the browser. Leaving a
server and deleting a command are recorded in the activity log. The panel needs outbound HTTPS to
`discord.com`, and the browser loads avatars from `cdn.discordapp.com`.
