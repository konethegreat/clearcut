# Deploying ClearCut for other people

_Kone Tshivhinda & Claude, 27 Oct 2026. For running ClearCut on a server so that **several accounts use it, each with their own Anthropic key**. On your own computer, [RUN-LOCALLY.md](RUN-LOCALLY.md) is all you need._

## Before anything else: you become the responsible party
Once other people upload their invoices to a server you run, **you** handle other people's personal information (POPIA), and the documents are sent to Anthropic outside South Africa to be read. That means an Information Officer, a privacy notice, an operator agreement with your users and a lawyer's review **before** the first outside user. Those are not built into the software and are on [LAUNCH.md](LAUNCH.md) section 3. Until they are done, run it for yourself and people who understand this.

## What you need
- A Linux server (a VM or a small VPS): **2 CPUs and 4 GB of memory at the least**, 8 GB if you expect many scanned pages (OCR is the heavy part), and disk for the files and the database. Hosting a small server costs roughly R150 to R400 a month at current prices (check them); the free options are in [ADR-0015](adr/0015-try-it-deployment.md) section 4.
- Docker and the Docker Compose plugin on it.
- A **domain name** pointing at the server (an `A` record), and ports **80 and 443** open to the internet. Nothing else should be open: the ClearCut server itself listens only on the server's own loopback address.
- Each user's own Anthropic key (they paste it in Settings; you never need one of your own).

## Set it up
1. Copy the project to the server (`git clone https://github.com/konethegreat/clearcut`).
2. Make the settings file `.env` in the project folder (see `.env.example`). The three passwords are **letters and digits only**, long, and different:
   ```text
   CLEARCUT_WORKER_TOKEN=<openssl rand -hex 24>
   DB_OWNER_PASSWORD=<openssl rand -hex 24>
   DB_APP_PASSWORD=<openssl rand -hex 24>
   CLEARCUT_DOMAIN=clearcut.yourdomain.example
   CLEARCUT_KEY_MODE=tenant-only
   CLEARCUT_RUNNER_CONCURRENCY=4
   ```
   **Do not set `ANTHROPIC_API_KEY`.** `tenant-only` means the server's own key is never used: every account brings its own.
3. Make the master key that seals each account's Anthropic key, and add it to `.env`:
   ```text
   docker compose run --rm admin generate-master-key
   ```
   It prints one line: put it in `.env` as `CLEARCUT_MASTER_KEY=<that line>`. **Keep a copy somewhere safe and apart from the server's backups.** Without it every stored key is unreadable; with it and the database together, anyone can read them.
4. Start everything, with HTTPS (Caddy gets and renews the certificate by itself):
   ```text
   docker compose --profile https up -d --build
   ```
   The first time it builds (several minutes). `https://clearcut.yourdomain.example/health` should answer `{"status":"ok",...}`.
5. Make the first account and its login (you, the operator, do this; users cannot):
   ```text
   docker compose run --rm admin create-tenant "Acme Advisory"
   docker compose run --rm admin create-key --tenant <the id it printed> --name "acme admin" --scopes documents:write,documents:read,keys:admin
   ```
   The key is printed **once**: hand it to that account's administrator over a safe channel. They open the page, paste it, go to **Settings** and paste their **own Anthropic key**; then they can upload. They can make further keys for their people with only the scopes needed.

## Check it works
- Open the page over HTTPS and sign in. Without an Anthropic key set, an uploaded document is analysed and then shows **Waiting for your Anthropic key** (and costs nothing).
- Set a key in Settings: the document is read within a minute.
- `curl -s -o /dev/null -w '%{http_code}' https://<domain>/v1/documents` answers `401` (a key is required).
- From outside, only ports 80 and 443 answer (`nmap` from another machine, or your provider's firewall page).

## Back up (and test the restore)
What holds state, and so what to back up:

| What | How | Note |
|---|---|---|
| The database | `docker compose exec -T db pg_dump -U clearcut clearcut > clearcut.sql` | invoices' facts, events, the audit log, **encrypted** Anthropic keys |
| The files | the `clearcut_files` volume (`docker run --rm -v clearcut_files:/data -v "$PWD":/out alpine tar czf /out/files.tgz -C /data .`) | the uploaded invoices themselves |
| `.env` and the **master key** | copy to an offline place | **never in the same place as the database dump** |
| Caddy's certificates | the `clearcut_caddy_data` volume | optional: they can be re-issued |

Encrypt the backups: they contain people's invoices. **A deleted document stays in older backups until they expire**, so keep a retention period you can state in your privacy notice. Restore into a spare machine at least once, before you need it.

## Upgrade
```text
git pull
docker compose --profile https up -d --build
```
The database tables are brought up to date automatically (the `migrate` step). Migrations only go forward: **take a backup first**.

## Keep it safe
- Encrypt the server's disk, allow SSH by key only, turn on automatic security updates.
- Keep the master key off the server's backups; keep `.env` readable only by you (`chmod 600 .env`).
- Tell each user to make a **dedicated Anthropic key with a monthly spend limit and an expiry** ([BYOK.md](BYOK.md)); ClearCut's own R5 a document and R100 a month are a second line.
- Watch `docker compose logs server`: it never contains invoice text or keys, only what happened.
- To remove a leaked ClearCut key: `docker compose run --rm admin revoke-key --tenant <id> --key <key id>`. To erase a document for good: `DELETE /v1/documents/{id}` (or the button on the page).
- Not built yet, so do by hand: rotating the database passwords, rotating the master key (re-enter each account's key afterwards), a tenant-management screen (the operator command is the tool), and email.

## Limits you should know
One server only; files are on a local disk volume (an S3-compatible store is on the checklist); no rate limiting at the edge beyond the keys being unguessable; the page and the API are the same address. The hosted free trial with no accounts ([ADR-0015](adr/0015-try-it-deployment.md)) is deliberately not built: it waits for a decision about who runs it.
