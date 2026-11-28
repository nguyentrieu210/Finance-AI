# Staff invitation and approval flow

Migrations 66–68 provide a seven-day, versioned invitation and approval path. An organization administrator creates a draft, then explicitly issues or revokes it. Issuance inserts one tenant/company-scoped delivery record in the durable outbox in the same transaction. The Rust API worker dispatches through Resend using the delivery UUID as an idempotency key, leases rows with `SKIP LOCKED`, retries transient errors with a bounded delay, and stores only static error categories. The API never logs the provider credential, response body, email address, or invite link.

The email points to the Nam Việt login. After Keycloak login, Finance AI claims an invitation only from the verified email on the token. The request body cannot choose email, issuer, or subject. Claiming creates a pending request and no membership. An organization administrator approves or rejects it with an expected version. Approval creates a requester membership only. The administration screen shows the separate invitation delivery state and pending approval queue.

`RESEND_API_KEY` and `RESEND_FROM_EMAIL` are read only from the API process environment. If either is missing or invalid, the dispatcher remains disabled and issued invitations remain visibly queued. This live VPS currently has neither setting configured, so email delivery is not active until an authorized sender and key are installed in the protected systemd environment. No test email is sent during deployment.

Effective grants remain a read-only preview with `enforced:false`; they do not yet authorize financial resources. Financial RLS and existing guards remain unchanged pending explicit dossier department and reviewer facts, lock integration, and end-to-end tests.

Migration 68 must be verified in isolated PostgreSQL before the live migration. It must not be considered live until the SQLx ledger, API binary, and health checks are verified.
