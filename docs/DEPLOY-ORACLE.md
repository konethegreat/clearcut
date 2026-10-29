# ClearCut on Oracle Cloud Always Free

_Kone Tshivhinda & Claude, 29 Oct 2026. The free-tier facts below were read on that date and can change; check the provider's own pages before relying on them. For the general deployment (what to back up, HTTPS, limits), read [DEPLOY.md](DEPLOY.md) first. This page only covers the free Oracle path._

## What this gives you
One always-on Arm virtual machine in Oracle's **Johannesburg** region, running the whole compose stack: the Rust server, the Python worker with Tesseract and PDFium, Postgres, the uploaded files and (if you choose it) Caddy for HTTPS. It is free under Oracle's Always Free allowance, with no cold starts and no sleeping, which matters because ClearCut keeps its rate limit and job runner in memory.

| What | Allowance (Always Free) | Note |
|---|---|---|
| Compute | Ampere A1, up to 4 OCPUs and 24 GB | Sometimes 2 OCPUs and 12 GB is all that is available; either is enough |
| Boot volume | 50 GB is plenty; the free block-storage total is 200 GB | The invoices and the database live here |
| Public IPv4 | One, reservable | A tunnel makes the address irrelevant |
| Outbound data | 10 TB a month | You will not come near it |

**Not free:** the Anthropic calls. The server runs in `tenant-only` mode, so every account pastes its own key and spends its own money. Your demo account spends yours.

## Before you start
- An Oracle Cloud account with a card for identity verification (a small temporary hold).
- A **domain name** for HTTPS. A cheap one is fine; a real domain in Cloudflare (free plan) is needed for the tunnel path. With no domain at all, the Caddy path can use a free `sslip.io` name made from the VM's IP (`169-0-0-1.sslip.io` for `169.0.0.1`), which needs ports 80 and 443 open like any other Caddy name.
- Your SSH public key on the computer you will use to log in.

## 1. Create the account and the VM
1. Sign up at cloud.oracle.com. Choose **South Africa Central (Johannesburg)** as the home region. The home region is permanent for Always Free resources and keeps the data in South Africa.
2. Compute, Instances, **Create instance**.
   - Name: `clearcut`.
   - Image: **Canonical Ubuntu 24.04** (the aarch64 image; Oracle labels it for the Arm shape).
   - Shape: **Ampere**, `VM.Standard.A1.Flex`, **4 OCPUs and 24 GB**. If it says there is no capacity, try 2 OCPUs and 12 GB, then try again later; free Arm capacity comes and goes.
   - Boot volume: 50 GB (choose a custom size if you like; stay within 200 GB).
   - SSH keys: paste your **public** key (`~/.ssh/id_ed25519.pub` on Windows).
   - Assign a public IPv4 address.
3. Wait for **RUNNING**, then note the public IP.
4. Reserve the IP (instance, Attached VNICs, the public IP, Edit, **Reserved**). Without this, stopping and starting the instance can change it. With the tunnel it does not matter.

**If Arm capacity stays full.** Johannesburg has one availability domain and its A1 pool can be empty for days. The hosted trial at https://145-241-100-85.sslip.io/ is running this way: a **trial-credit** `VM.Standard.E5.Flex`, 2 OCPUs and 8 GB, on the x86 Ubuntu 24.04 image, with the same steps from section 3 onward. It costs roughly R27 a day out of the trial credit, and the credit and the trial's length are Oracle's to set, so read your own account's terms: a trial is a way to be live now, not a free host, and the paid instance does not outlive it. Keep asking for the A1: `oci compute instance launch` in a loop until it answers with a running instance, then move the stack (clone, `pg_dump` and restore, copy the uploaded files, repoint DNS) and delete the trial VM. `deploy/oracle-install.sh` does not care which shape it starts on.

## 2. Choose how HTTPS reaches it
Two ways. The tunnel is the safer default because nothing but SSH is open to the internet.

### A. Cloudflare Tunnel (recommended)
- Keep the default security list: **SSH only**.
- The tunnel creates the public hostname itself; no DNS record points at the VM.
- You need a domain using Cloudflare's nameservers (free plan).

### B. Caddy with a public port
- Add ingress rules to the subnet's security list for **TCP 80 and 443 from 0.0.0.0/0**.
- Point an `A` record at the reserved public IP, or skip DNS and use the `sslip.io` name above.
- Oracle's Ubuntu image also filters ports in its own firewall. Allow them:
  ```bash
  sudo iptables -I INPUT 6 -m state --state NEW -p tcp --dport 80 -j ACCEPT
  sudo iptables -I INPUT 6 -m state --state NEW -p tcp --dport 443 -j ACCEPT
  sudo netfilter-persistent save
  ```

## 3. Prepare Ubuntu
```bash
ssh ubuntu@<the public IP>
sudo apt update && sudo apt full-upgrade -y
sudo apt install -y ca-certificates curl gnupg unattended-upgrades
sudo systemctl enable --now unattended-upgrades
sudo timedatectl set-timezone Africa/Johannesburg
```
Keep swap **off** so a stored Anthropic key is never written to disk:
```bash
sudo swapoff -a
sudo sed -i '/ swap / s/^/#/' /etc/fstab
```
Install Docker from Docker's own repository (it has arm64 packages):
```bash
sudo install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg
sudo chmod a+r /etc/apt/keyrings/docker.gpg
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo $VERSION_CODENAME) stable" | sudo tee /etc/apt/sources.list.d/docker.list
sudo apt update
sudo apt install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
sudo usermod -aG docker ubuntu
```
Log out and back in so the group applies.

## 4. Put ClearCut on it
```bash
git clone https://github.com/konethegreat/clearcut.git
cd clearcut
```
Make `.env` (see [.env.example](../.env.example)); the three passwords are letters and digits only, long, and different:
```text
CLEARCUT_WORKER_TOKEN=<openssl rand -hex 24>
DB_OWNER_PASSWORD=<openssl rand -hex 24>
DB_APP_PASSWORD=<openssl rand -hex 24>
CLEARCUT_KEY_MODE=tenant-only
CLEARCUT_RUNNER_CONCURRENCY=2
CLEARCUT_RETENTION_DAYS=365
```
Make the master key, then add the line it prints as `CLEARCUT_MASTER_KEY=`:
```bash
docker compose run --rm admin generate-master-key
```
Keep a copy of the master key somewhere apart from the server and its backups. Without it every stored tenant key is unreadable; with it and the database together, anyone can read them.

Start it:
```bash
docker compose up -d --build          # tunnel path
# or, for Caddy with a domain and ports 80/443 open:
# echo 'CLEARCUT_DOMAIN=clearcut.yourdomain.example' >> .env
# docker compose --profile https up -d --build
```
The first build compiles the Rust server on the VM and takes several minutes. Check it:
```bash
curl -s http://127.0.0.1:8099/health
docker compose ps
```

## 5. Connect the tunnel (path A)
Install `cloudflared` (arm64):
```bash
sudo mkdir -p --mode=0755 /usr/share/keyrings
curl -fsSL https://pkg.cloudflare.com/cloudflare-main.gpg | sudo tee /usr/share/keyrings/cloudflare-main.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/cloudflare-main.gpg] https://pkg.cloudflare.com/cloudflared any main" | sudo tee /etc/apt/sources.list.d/cloudflared.list
sudo apt update && sudo apt install -y cloudflared
```
In the Cloudflare dashboard:
1. Zero Trust, Networks, **Tunnels**, Create a tunnel, **Cloudflared**, give it a name.
2. Copy the install command it shows and run it on the VM: `sudo cloudflared service install <token>`.
3. Open the tunnel, **Public hostname**, add one: subdomain `clearcut`, your domain, service type **HTTP**, URL `127.0.0.1:8099`.
4. SSL/TLS, Overview: **Full**. Rules: turn on **Always Use HTTPS**.

Then `https://clearcut.yourdomain.example/health` answers `{"status":"ok",...}`, and only port 22 is open to the internet. Cloudflare's free plan allows request bodies up to 100 MB; ClearCut refuses uploads over 64 MiB, so that is not a limit you will hit.

## 6. Make the first account
```bash
docker compose run --rm admin create-tenant "Kone (demo)"
docker compose run --rm admin create-key --tenant <the id it printed> \
  --name "kone admin" --scopes documents:write,documents:read,keys:admin
```
The key is printed **once**. Open the site, paste it, go to Settings and paste your **own Anthropic key**, made with a monthly spend limit and an expiry ([BYOK.md](BYOK.md)). Then upload a test invoice.

## 7. Back up and upgrade
The state, the backup commands and the restore test are in [DEPLOY.md](DEPLOY.md#back-up-and-test-the-restore). On Oracle, also:
- Turn on a **block volume backup policy** for the boot volume (console, Block Storage, Boot Volumes, Backups). It is a second line, not a replacement.
- Copy an encrypted `pg_dump` and the files archive off the VM regularly (your laptop or Oracle Object Storage's free allowance). **Keep `.env` and the master key somewhere else again**, never with the dump.
- Upgrade: `cd ~/clearcut && git pull && docker compose up -d --build`. Take a backup first; migrations only go forward.

## 8. What we checked about Arm
On 29 Oct 2026, with Docker Buildx on an amd64 machine:
- The **worker** image builds for `linux/arm64`, starts, and `/health` answers `{"status":"ok",...,"capabilities":["inspect","text","render","mask","ocr"]}`. That proves the parts that matter on Arm: Tesseract (English and Afrikaans), PDFium (`pypdfium2` aarch64 wheel) and Pillow.
- The **server** image builds for `linux/arm64` as well (pure Rust; the release build is just slower under emulation).

The VM builds the same Dockerfiles natively, which is faster than the emulated check. If you change a base image or add a native Python or system package, run the check again before deploying:
```bash
docker buildx build --platform linux/arm64 -f services/worker/Dockerfile -t clearcut-worker:arm64 --load services/worker
docker buildx build --platform linux/arm64 -t clearcut-server:arm64 --load .
```

## 9. When something goes wrong
- **"Out of host capacity"** when creating the A1 instance: try 2 OCPUs and 12 GB, another availability domain if the region offers one, or retry later. This is the usual free-tier blocker, not a sign of a mistake.
- **Build is slow**: the first Rust release build on a free VM is minutes, not seconds. Later builds reuse the cache.
- **Memory pressure**: `free -h` and `docker stats`. Lower `CLEARCUT_RUNNER_CONCURRENCY` to 1; OCR is the heavy part. 12 GB is enough for one document at a time.
- **The public IP changed** (Caddy path): reserve it, or move to the tunnel.
- **Oracle reclaims an idle Always Free instance**: Oracle can reclaim Always Free compute that looks idle over a long period. A service with regular traffic should be fine, but keep off-site backups, and a paid upgrade removes the risk.
- **No site over the tunnel**: `sudo systemctl status cloudflared`, then check the public hostname points at `127.0.0.1:8099` and the server is listening (`curl` on the VM).

## 10. Before anyone else's invoices
Deploying for yourself and people who understand this is fine now. Once other people upload their invoices, **you** become the responsible party under POPIA: an Information Officer, a privacy notice, an operator agreement and a lawyer's review are needed first ([DEPLOY.md](DEPLOY.md#before-anything-else-you-become-the-responsible-party), [LAUNCH.md](LAUNCH.md) section 3). Set the retention period you can justify in `CLEARCUT_RETENTION_DAYS` and say the same period in the notice.
