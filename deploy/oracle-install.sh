#!/usr/bin/env bash
# Written by Kone Tshivhinda & Claude
# Sets ClearCut up on an Oracle Cloud Always Free VM (Ampere, Ubuntu 24.04).
# Run it as the regular "ubuntu" user, from inside the cloned repository:
#
#   bash deploy/oracle-install.sh [hostname]
#
# hostname: the public HTTPS name that already points at this machine, for example
#   clearcut.example.com, or a free <ip-with-dashes>.sslip.io name. Without it the stack
#   starts privately on 127.0.0.1:8099; you can add a Cloudflare Tunnel afterwards.
#
# Safe to run again: it keeps .env and the master key, and only creates the first account
# when no admin key file exists. It asks for sudo, never for your Anthropic key (paste that
# in Settings once the site is up).
set -euo pipefail

DOMAIN="${1:-}"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEY_FILE="$HOME/clearcut-admin-key.txt"

say() { printf '\n==> %s\n' "$*"; }
die() { printf '\nerror: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -ne 0 ] || die "run this as the ubuntu user, not root; it uses sudo where it needs to"
command -v sudo >/dev/null || die "sudo is required"
[ -f "$REPO_DIR/docker-compose.yml" ] || die "run this from inside the ClearCut repository"

say "System packages and time"
sudo apt-get update -y
sudo apt-get install -y ca-certificates curl gnupg openssl unattended-upgrades
sudo timedatectl set-timezone Africa/Johannesburg || true

# A stored Anthropic key must never be paged to disk.
if [ -n "$(swapon --show 2>/dev/null)" ]; then
    say "Turning swap off"
    sudo swapoff -a
    sudo sed -i '/ swap / s/^/#/' /etc/fstab
fi

if ! command -v docker >/dev/null; then
    say "Installing Docker"
    sudo install -m 0755 -d /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg
    sudo chmod a+r /etc/apt/keyrings/docker.gpg
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo "$VERSION_CODENAME") stable" | sudo tee /etc/apt/sources.list.d/docker.list >/dev/null
    sudo apt-get update -y
    sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
    sudo usermod -aG docker "$USER"
fi
if docker info >/dev/null 2>&1; then DOCKER="docker"; else DOCKER="sudo docker"; fi

cd "$REPO_DIR"

if [ ! -f .env ]; then
    say "Writing .env with fresh secrets"
    umask 077
    {
        echo "CLEARCUT_WORKER_TOKEN=$(openssl rand -hex 24)"
        echo "DB_OWNER_PASSWORD=$(openssl rand -hex 24)"
        echo "DB_APP_PASSWORD=$(openssl rand -hex 24)"
        echo "CLEARCUT_KEY_MODE=tenant-only"
        echo "CLEARCUT_RUNNER_CONCURRENCY=2"
        echo "CLEARCUT_RETENTION_DAYS=365"
        if [ -n "$DOMAIN" ]; then echo "CLEARCUT_DOMAIN=$DOMAIN"; fi
    } > .env
    chmod 600 .env
else
    say ".env already exists; leaving its secrets alone"
    if [ -n "$DOMAIN" ] && ! grep -q '^CLEARCUT_DOMAIN=' .env; then
        umask 077
        echo "CLEARCUT_DOMAIN=$DOMAIN" >> .env
    fi
fi

if ! grep -q '^CLEARCUT_MASTER_KEY=' .env; then
    say "Making the master key (keep a copy somewhere apart from the server and its backups)"
    MASTER=$($DOCKER compose run --rm --no-deps -T admin generate-master-key 2>/dev/null | tail -n1)
    [ -n "$MASTER" ] || die "the master key came back empty; is the database healthy?"
    umask 077
    printf 'CLEARCUT_MASTER_KEY=%s\n' "$MASTER" >> .env
    chmod 600 .env
fi

if [ -n "$DOMAIN" ]; then
    say "Starting ClearCut with HTTPS for $DOMAIN (the first build takes minutes)"
    $DOCKER compose --profile https up -d --build
    for port in 80 443; do
        sudo iptables -C INPUT -p tcp --dport "$port" -j ACCEPT 2>/dev/null \
            || sudo iptables -I INPUT 1 -p tcp --dport "$port" -j ACCEPT
    done
    if command -v netfilter-persistent >/dev/null; then sudo netfilter-persistent save >/dev/null; fi
    echo "Reminder: the Oracle security list must also allow TCP 80 and 443 from 0.0.0.0/0."
else
    say "Starting ClearCut privately on this machine (127.0.0.1:8099)"
    $DOCKER compose up -d --build
fi

say "Waiting for the server"
ready=""
for _ in $(seq 1 90); do
    if curl -fsS http://127.0.0.1:8099/health >/dev/null 2>&1; then ready=yes; break; fi
    sleep 2
done
[ -n "$ready" ] || die "the server did not answer; see: $DOCKER compose logs server"
curl -fsS http://127.0.0.1:8099/health; echo

if [ ! -f "$KEY_FILE" ]; then
    say "Making the first account and its key"
    TENANT=$($DOCKER compose run --rm -T admin create-tenant "Kone (demo)" 2>/dev/null | tail -n1)
    [ -n "$TENANT" ] || die "the tenant id came back empty"
    KEY=$($DOCKER compose run --rm -T admin create-key --tenant "$TENANT" \
        --name "kone admin" --scopes documents:write,documents:read,keys:admin 2>/dev/null | tail -n1)
    [ -n "$KEY" ] || die "the key came back empty"
    umask 077
    printf '%s\n' "$KEY" > "$KEY_FILE"
    chmod 600 "$KEY_FILE"
    echo "The admin key was printed once and saved to $KEY_FILE (move it to a password manager)."
else
    say "An admin key file already exists; not making another account"
fi

if [ -n "$DOMAIN" ]; then
    say "Done. Open https://$DOMAIN/ , sign in with the key in $KEY_FILE, then Settings and paste your own Anthropic key (with a spend limit)."
else
    IP=$(curl -fsS https://ifconfig.me 2>/dev/null || true)
    say "Done, privately. For HTTPS without a domain, open TCP 80 and 443 in the Oracle security list, then run:"
    if [ -n "$IP" ]; then
        echo "  bash deploy/oracle-install.sh ${IP//./-}.sslip.io"
    else
        echo "  bash deploy/oracle-install.sh <your-ip-with-dashes>.sslip.io"
    fi
    echo "Or point a Cloudflare Tunnel at http://127.0.0.1:8099 (docs/DEPLOY-ORACLE.md)."
fi
