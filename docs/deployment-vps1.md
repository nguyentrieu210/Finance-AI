# VPS1 Finance deployment

The deployment entrypoint is scripts/deploy_vps1.sh. It operates only on the Finance API, Finance frontend static release, Finance PostgreSQL, and the Nam Viet Keycloak health endpoint.

Run the read-only preflight first:

    bash scripts/deploy_vps1.sh --preflight

Deploy only from a clean local main that matches origin/main:

    bash scripts/deploy_vps1.sh --deploy

## What it checks and runs

- Confirms GitHub main, the VPS checkout, and the working tree agree.
- Checks the Finance API/frontend systemd units, Nginx configuration, Finance PostgreSQL, Keycloak realm metadata, API readiness, and available disk. Preflight requires a 2 GiB build reserve plus the live database size and backup reserve.
- Reuses `/opt/namviet-finance-build` as the Cargo target cache and `/root/.cargo` for crate downloads. Cargo runs in a short-lived container from the pinned `rust:1.85.1-slim-bookworm@sha256:9f841bbe9e7d8e37ceb96ed907265a3a0df7f44e3737d0b100e7907a679acb36` image, so no Rust toolchain is installed on the VPS host. It does not run `cargo clean` or reinstall npm dependencies.
- Runs Rust format/unit checks, table registry validation, isolated migration/restore tests, frontend API contract checks, frontend tests, and the production build.
- Creates a root-only custom-format PostgreSQL backup and verifies its archive before applying SQLx migrations.
- Installs hashed frontend assets before atomically switching the live index.html; swaps the API binary while retaining the prior binary for rollback.
- Restarts only namviet-finance-api.service and finance-web-dev.service. It tests Nginx configuration but does not reload Nginx or restart PostgreSQL, Keycloak, OpenVietGIS, or other services.
- Checks local/public readiness, the existing Keycloak login redirect, and that unauthenticated membership requests remain denied.

The script reads PostgreSQL credentials only from /etc/namviet-finance/db.env; it never writes credentials to the repository or prints them. Resend delivery is not part of the smoke test and remains disabled until the API's protected environment contains RESEND_API_KEY and RESEND_FROM_EMAIL.

If a post-switch health check fails, the script restores the previous API binary and frontend index. A successful SQL migration is left in place because blindly restoring a database after application writes could discard data. The pre-migration backup remains in /var/backups/finance-ai for a controlled database restore.

A successful deployment does not constitute G01/G04 business or security sign-off and does not activate unapproved financial authorization policies.
