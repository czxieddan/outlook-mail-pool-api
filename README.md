# Outlook Mail Pool API

Rust service for importing Outlook/Hotmail mailboxes and exposing a mail-provider API that `gpt-register-oss` can consume.

## Features

- Bulk import `email,password` or `email,password,imap_host,imap_port`.
- Async Axum HTTP API with Tokio worker limits for IMAP refreshes.
- SQLite persistence for accounts, leases, and fetched messages.
- `gpt-register-oss` compatible endpoints:
  - `GET /api/latest?address=<email>` for `self_hosted_mail_api`.
  - `POST /v1/accounts`, `GET /v1/messages`, `GET /v1/messages/{id}` for `yyds_mail` style leasing.
- Built-in management page at `/`.

## Run Locally

```bash
cp .env.example .env
cargo run
```

Open `http://127.0.0.1:8098/` and enter `MAIL_POOL_API_KEY`.

## Configure gpt-register-oss

Recommended for imported fixed Outlook accounts:

```json
"mail": {
  "provider": "yyds_mail",
  "api_base": "http://127.0.0.1:8098/v1",
  "api_key": "YOUR_MAIL_POOL_API_KEY",
  "domain": "outlook.com",
  "otp_timeout_seconds": 120,
  "poll_interval_seconds": 3
},
"yyds_mail": {
  "api_base": "http://127.0.0.1:8098/v1",
  "api_key": "YOUR_MAIL_POOL_API_KEY",
  "domain": "outlook.com"
}
```

This makes `gpt-register-oss` lease one imported mailbox per registration attempt.

The legacy self-hosted API is also available:

```json
"mail": {
  "provider": "self_hosted_mail_api",
  "api_base": "http://127.0.0.1:8098",
  "api_key": "YOUR_MAIL_POOL_API_KEY",
  "domain": "outlook.com"
}
```

Use this only when `gpt-register-oss` is already creating addresses that exist in the imported pool.

## Import Format

```text
user1@outlook.com,password1
user2@hotmail.com:password2
user3@outlook.com,password3,outlook.office365.com,993
```

Outlook defaults to `outlook.office365.com:993`.
