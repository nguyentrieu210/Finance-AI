#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
export GIT_TERMINAL_PROMPT=0
export LC_ALL=C

readonly REPO=/opt/finance-ai
readonly BUILD_ROOT=/opt/namviet-finance-build
readonly RUST_IMAGE=rust:1.85.1-slim-bookworm@sha256:9f841bbe9e7d8e37ceb96ed907265a3a0df7f44e3737d0b100e7907a679acb36
readonly CARGO_CACHE=/root/.cargo
readonly API_BINARY=/usr/local/lib/namviet-finance/finance-api
readonly API_UNIT=namviet-finance-api.service
readonly WEB_UNIT=finance-web-dev.service
readonly DB_CONTAINER=namviet-finance-db
readonly KEYCLOAK_CONTAINER=namviet-auth
readonly DB_ENV_FILE=/etc/namviet-finance/db.env
readonly NGINX_SITE=/etc/nginx/sites-enabled/namviet.gpcds.site.conf
readonly BACKUP_DIR=/var/backups/finance-ai
readonly LOCAL_READY=http://127.0.0.1:18194/health/ready
readonly PUBLIC_READY=https://namviet.gpcds.site/health/ready
readonly PUBLIC_OIDC=https://namviet.gpcds.site/realms/namviet/.well-known/openid-configuration

mode=""
if (( $# > 0 )); then mode="$1"; fi
if [[ "$mode" != "--preflight" && "$mode" != "--deploy" ]]; then
  printf 'Usage: %s --preflight|--deploy\n' "$0" >&2
  exit 64
fi

log() { printf '[finance-deploy] %s\n' "$1"; }
die() {
  log "FAILED: $1"
  if declare -F restore_on_error >/dev/null 2>&1; then
    restore_on_error 1
  fi
  exit 1
}

[[ "$(id -u)" == 0 ]] || die "run as root"
[[ -d "$REPO/.git" ]] || die "repository not found at the configured path"
cd "$REPO"

check_checkout() {
  git fetch origin main >/dev/null 2>&1 || die "cannot refresh origin/main"
  [[ "$(git branch --show-current)" == main ]] || die "deployment requires branch main"
  [[ -z "$(git status --porcelain)" ]] || die "working tree is dirty"
  [[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || die "local main does not match GitHub main"
  log "GitHub main and local main match at $(git rev-parse --short HEAD)"
}

https_root() {
  awk '
    /^server[[:space:]]*\{/ { inside=1; tls=0; candidate="" }
    inside && /listen[[:space:]]+443[[:space:]]+ssl/ { tls=1 }
    inside && $1=="root" { candidate=$2; sub(/;$/, "", candidate) }
    inside && /^}/ { if (tls) { print candidate; exit } inside=0 }
  ' "$NGINX_SITE"
}

check_runtime() {
  [[ -r "$DB_ENV_FILE" ]] || die "protected PostgreSQL environment file is unavailable"
  [[ "$(stat -c '%u' "$DB_ENV_FILE")" == 0 ]] || die "PostgreSQL environment file is not root-owned"
  local db_mode
  db_mode="$(stat -c '%a' "$DB_ENV_FILE")"
  [[ "$db_mode" == 600 || "$db_mode" == 400 ]] || die "PostgreSQL environment file permissions are too broad"
  docker inspect --format '{{.State.Running}}' "$DB_CONTAINER" 2>/dev/null | grep -qx true || die "Finance PostgreSQL container is not running"
  docker inspect --format '{{.State.Running}}' "$KEYCLOAK_CONTAINER" 2>/dev/null | grep -qx true || die "Nam Viet Keycloak container is not running"
  docker exec "$DB_CONTAINER" sh -lc 'pg_isready -U "$POSTGRES_USER" -d "$POSTGRES_DB"' >/dev/null || die "Finance PostgreSQL is not ready"
  systemctl is-active --quiet "$API_UNIT" || die "Finance API systemd unit is not active"
  systemctl is-active --quiet "$WEB_UNIT" || die "Finance frontend systemd unit is not active"
  systemctl is-active --quiet nginx.service || die "Nginx is not active"
  nginx -t >/dev/null 2>&1 || die "Nginx configuration test failed"
  curl -fsS -o /dev/null "$PUBLIC_OIDC" || die "Keycloak realm metadata is unavailable"
  curl -fsS -o /dev/null "$LOCAL_READY" || die "Finance API readiness check failed"
  curl -fsS -o /dev/null "$PUBLIC_READY" || die "public Finance API readiness check failed"
  local root_path
  root_path="$(https_root)"
  [[ "$root_path" == /var/www/finance-ai-releases/* && -d "$root_path/assets" ]] || die "Nginx does not point at the expected Finance static release"
  log "Finance API, frontend, PostgreSQL, Keycloak and Nginx checks passed"
}

check_disk() {
  local available
  available="$(df -B1 -P / | awk 'NR==2 {print $4}')"
  [[ "$available" =~ ^[0-9]+$ ]] || die "could not read available filesystem space"
  local db_size required
  db_size="$(docker exec "$DB_CONTAINER" sh -lc 'psql -X -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atc "select pg_database_size(current_database())"')"
  [[ "$db_size" =~ ^[0-9]+$ ]] || die "could not read Finance database size for backup-space planning"
  required=$((db_size + 268435456))
  if (( required < 2147483648 )); then required=2147483648; fi
  (( available >= required )) || die "available space is below the safe build and database-backup reserve"
  log "filesystem has the 2 GiB build reserve and database backup reserve"
}

check_migration_space() {
  local available
  available="$(df -B1 -P / | awk 'NR==2 {print $4}')"
  [[ "$available" =~ ^[0-9]+$ ]] || die "could not read post-build free space"
  local db_size
  db_size="$(docker exec "$DB_CONTAINER" sh -lc 'psql -X -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atc "select pg_database_size(current_database())"')"
  [[ "$db_size" =~ ^[0-9]+$ ]] || die "could not read Finance database size after build"
  (( available >= db_size + 268435456 )) || die "less than database size plus 256 MiB backup reserve remains; refusing database migration"
}

preflight() {
  check_checkout
  check_runtime
  check_disk
  command -v docker >/dev/null || die "Docker is unavailable"
  docker image inspect "$RUST_IMAGE" >/dev/null 2>&1 || die "pinned Rust build image is unavailable"
  [[ -d "$CARGO_CACHE" ]] || die "Cargo cache directory is unavailable"
  command -v npm >/dev/null || die "npm is unavailable"
  command -v python3 >/dev/null || die "Python 3 is unavailable"
}

timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
backup_db=""
local_db_tmp=""
remote_db_tmp=""
old_api=""
old_index=""
index_tmp=""
api_old_moved=0
index_swapped=0
deploy_started=0
migration_applied=0
build_container=""

start_build_container() {
  build_container="namviet-finance-build-${timestamp}-${BASHPID}"
  docker run -d --name "$build_container" -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/target -v "$CARGO_CACHE:/cache/cargo" -v "$REPO:/workspace" -v "$BUILD_ROOT:/target" -w /workspace "$RUST_IMAGE" tail -f /dev/null >/dev/null || die "could not start the pinned Rust build container"
}

cargo() {
  [[ -n "$build_container" ]] || die "Rust build container is not initialized"
  docker exec "$build_container" cargo "$@"
}

cleanup_build_container() {
  if [[ -n "$build_container" ]]; then
    docker rm -f "$build_container" >/dev/null 2>&1 || true
    build_container=""
  fi
}

restore_on_error() {
  local rc=${1:-$?}
  trap - ERR
  cleanup_build_container
  if (( deploy_started )); then
    log "deployment health check failed; restoring the previous API and frontend index"
    if (( index_swapped )) && [[ -s "$old_index" ]]; then
      install -m 0644 "$old_index" "$index_tmp.rollback" || true
      mv -f "$index_tmp.rollback" "$(https_root)/index.html" || true
    fi
    if (( api_old_moved )) && [[ -s "$old_api" ]]; then
      if [[ -f "$API_BINARY" ]]; then
        mv -f "$API_BINARY" "$BUILD_ROOT/debug/finance-api.failed-$timestamp" || true
      fi
      mv -f "$old_api" "$API_BINARY" || true
      chown root:root "$API_BINARY" 2>/dev/null || true
      chmod 0755 "$API_BINARY" 2>/dev/null || true
      systemctl restart "$API_UNIT" || true
    fi
    systemctl restart "$WEB_UNIT" || true
  fi
  [[ -z "$local_db_tmp" || ! -f "$local_db_tmp" ]] || rm -f -- "$local_db_tmp"
  [[ -z "$remote_db_tmp" ]] || docker exec "$DB_CONTAINER" rm -f "$remote_db_tmp" >/dev/null 2>&1 || true
  [[ -z "$index_tmp" || ! -f "$index_tmp" ]] || rm -f -- "$index_tmp"
  if (( migration_applied )); then
    log "additive database migration remains applied; verified pre-migration backup: $backup_db"
  fi
  exit "$rc"
}
trap restore_on_error ERR
trap cleanup_build_container EXIT

preflight
if [[ "$mode" == "--preflight" ]]; then
  trap - ERR
  log "preflight complete; no build, backup, migration or restart was performed"
  exit 0
fi

start_build_container
cargo fmt --all -- --check
cargo test --locked -p finance-api --lib
for fingerprint in "$BUILD_ROOT"/debug/.fingerprint/finance-api-*/test-lib-finance_api.json; do
  [[ -f "$fingerprint" ]] || continue
  fingerprint_dir="$(basename "$(dirname "$fingerprint")")"
  fingerprint_hash="$(printf '%s' "$fingerprint_dir" | sed 's/^finance-api-//')"
  test_binary="$BUILD_ROOT/debug/deps/finance_api-$fingerprint_hash"
  [[ -x "$test_binary" ]] && rm -f -- "$test_binary"
done

python3 scripts/validate_table_registry.py
cargo build --locked -p finance-migrate --bin finance-migrate
MIGRATOR="$BUILD_ROOT/debug/finance-migrate"
for duplicate in "$BUILD_ROOT"/debug/deps/finance_migrate-*; do
  [[ -x "$duplicate" ]] || continue
  cmp -s "$MIGRATOR" "$duplicate" && rm -f -- "$duplicate"
done
FINANCE_MIGRATOR_BIN="$MIGRATOR" bash scripts/test_migration_baseline.sh
FINANCE_MIGRATOR_BIN="$MIGRATOR" bash scripts/test_restore_drill.sh
(
  cd apps/web
  npm run check:api
  npm test
  VITE_STAFF_ADMIN_ENABLED=true npm run build
)

local_api_build_space="$(df -B1 -P / | awk 'NR==2 {print $4}')"
[[ "$local_api_build_space" =~ ^[0-9]+$ ]] || die "could not read pre-API-build free space"
(( local_api_build_space >= 1073741824 )) || die "less than 1 GiB remains before API build; refusing to consume the filesystem reserve"
cargo build --locked -p finance-api --bin finance-api
readonly NEW_API="$BUILD_ROOT/debug/finance-api"
[[ -x "$NEW_API" && -x "$MIGRATOR" ]] || die "release binaries were not produced"
if ldd "$NEW_API" 2>&1 | grep -q 'not found'; then die "API binary has missing shared libraries"; fi
for duplicate in "$BUILD_ROOT"/debug/deps/finance_api-*; do
  [[ -x "$duplicate" ]] || continue
  cmp -s "$NEW_API" "$duplicate" && rm -f -- "$duplicate"
done
for duplicate in "$BUILD_ROOT"/debug/deps/finance_migrate-*; do
  [[ -x "$duplicate" ]] || continue
  cmp -s "$MIGRATOR" "$duplicate" && rm -f -- "$duplicate"
done
cleanup_build_container
check_migration_space

root_path="$(https_root)"
release="$root_path"
[[ -s "$release/index.html" && -d "$release/assets" ]] || die "live frontend release is incomplete"
[[ -s apps/web/dist/index.html && -n "$(find apps/web/dist/assets -maxdepth 1 -type f -print -quit)" ]] || die "frontend build output is incomplete"
mkdir -p "$BACKUP_DIR"
chmod 0700 "$BACKUP_DIR"
chown root:root "$BACKUP_DIR"
backup_db="$BACKUP_DIR/finance-before-$timestamp.dump"
local_db_tmp="$BACKUP_DIR/.finance-before-$timestamp.dump.partial"
remote_db_tmp="/tmp/finance-before-$timestamp.dump"
old_api="$BACKUP_DIR/finance-api-before-$timestamp"
old_index="$BACKUP_DIR/frontend-index-before-$timestamp.html"
index_tmp="$release/.index.html-$timestamp"
[[ ! -e "$backup_db" && ! -e "$old_api" && ! -e "$old_index" ]] || die "timestamp collision in backup directory"

log "creating and validating a protected PostgreSQL backup"
docker exec "$DB_CONTAINER" sh -lc 'pg_dump -Fc -U "$POSTGRES_USER" -d "$POSTGRES_DB"' > "$local_db_tmp"
[[ -s "$local_db_tmp" ]] || die "PostgreSQL backup is empty"
chmod 0600 "$local_db_tmp"
docker cp "$local_db_tmp" "$DB_CONTAINER:$remote_db_tmp" >/dev/null
docker exec "$DB_CONTAINER" pg_restore --list "$remote_db_tmp" >/dev/null
docker exec "$DB_CONTAINER" rm -f "$remote_db_tmp" >/dev/null
mv "$local_db_tmp" "$backup_db"
log "database backup verified at $backup_db"

set -a
. "$DB_ENV_FILE"
set +a
db_port="$(docker port "$DB_CONTAINER" 5432/tcp | awk -F: '$1=="127.0.0.1" {print $NF}')"
[[ "$db_port" =~ ^[0-9]+$ ]] || die "Finance PostgreSQL is not bound to loopback"
FINANCE_MIGRATION_DATABASE_URL="$(FINANCE_DB_PORT="$db_port" python3 -c 'import os; from urllib.parse import quote; u=quote(os.environ["POSTGRES_USER"],safe=""); p=quote(os.environ["POSTGRES_PASSWORD"],safe=""); d=quote(os.environ["POSTGRES_DB"],safe=""); print("postgres://{}:{}@127.0.0.1:{}/{}".format(u,p,os.environ["FINANCE_DB_PORT"],d))')"
export FINANCE_MIGRATION_DATABASE_URL
unset POSTGRES_PASSWORD POSTGRES_USER POSTGRES_DB
"$MIGRATOR" --apply
unset FINANCE_MIGRATION_DATABASE_URL
migration_applied=1
ledger="$(docker exec "$DB_CONTAINER" sh -lc 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atc "select count(*)||chr(58)||max(version) from public._sqlx_migrations where success"')"
[[ "$ledger" =~ ^[0-9]+:20[0-9]{12}$ ]] || die "migration ledger verification failed"
log "migration ledger verified at $ledger"

for asset in apps/web/dist/assets/*; do
  [[ -f "$asset" ]] || continue
  install -m 0644 "$asset" "$release/assets/$(basename "$asset")"
done
install -m 0644 "$release/index.html" "$old_index"
install -m 0644 apps/web/dist/index.html "$index_tmp"
sync -f "$index_tmp"

deploy_started=1
mv -f "$index_tmp" "$release/index.html"
index_swapped=1
sync -f "$release/index.html"
mv "$API_BINARY" "$old_api"
api_old_moved=1
mv "$NEW_API" "$API_BINARY"
chown root:root "$API_BINARY"
chmod 0755 "$API_BINARY"

systemctl restart "$API_UNIT"
for attempt in $(seq 1 30); do
  if curl -fsS -o /dev/null "$LOCAL_READY"; then break; fi
  sleep 1
done
curl -fsS -o /dev/null "$LOCAL_READY"
systemctl restart "$WEB_UNIT"
systemctl is-active --quiet "$API_UNIT"
systemctl is-active --quiet "$WEB_UNIT"
systemctl is-active --quiet nginx.service
nginx -t >/dev/null 2>&1
curl -fsS -o /dev/null "$PUBLIC_READY"
[[ "$(curl -sS -o /dev/null -w '%{http_code}' 'https://namviet.gpcds.site/auth/start?rd=%2Finvite')" == 302 ]] || die "Keycloak login redirect smoke test failed"
[[ "$(curl -sS -o /dev/null -w '%{http_code}' https://namviet.gpcds.site/v1/memberships)" == 401 ]] || die "unauthenticated membership request was not denied"
git fetch origin main >/dev/null 2>&1
[[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || die "GitHub branch changed during deployment"
trap - ERR
log "deployment passed; API and frontend are healthy; database backup: $backup_db"
log "rollback binaries and prior index retained under $BACKUP_DIR"
