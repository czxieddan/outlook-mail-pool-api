use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};
use uuid::Uuid;

use crate::importer::ImportedAccount;

#[derive(Debug, Clone, Serialize)]
pub struct AccountRecord {
    pub id: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: i64,
    pub status: String,
    pub last_error: String,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct LeasedAccount {
    pub id: String,
    pub email: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: i64,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct StoredMail {
    pub id: String,
    pub account_email: String,
    pub subject: String,
    pub text: String,
    pub html: String,
    pub from_addr: String,
    pub received_at: DateTime<Utc>,
    pub verification_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AccountSecret {
    pub id: String,
    pub email: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: i64,
}

#[derive(Debug, Clone)]
pub struct NewMail {
    pub id: String,
    pub account_email: String,
    pub subject: String,
    pub text: String,
    pub html: String,
    pub from_addr: String,
    pub received_at: DateTime<Utc>,
    pub verification_code: Option<String>,
    pub raw_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub total: i64,
    pub idle: i64,
    pub leased: i64,
    pub ready: i64,
    pub error: i64,
    pub messages: i64,
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect(database_url)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    async fn migrate(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL UNIQUE,
                password TEXT NOT NULL,
                imap_host TEXT NOT NULL,
                imap_port INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'idle',
                lease_token TEXT NOT NULL DEFAULT '',
                lease_until TEXT,
                last_error TEXT NOT NULL DEFAULT '',
                last_checked_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                account_email TEXT NOT NULL,
                subject TEXT NOT NULL DEFAULT '',
                text TEXT NOT NULL DEFAULT '',
                html TEXT NOT NULL DEFAULT '',
                from_addr TEXT NOT NULL DEFAULT '',
                received_at TEXT NOT NULL,
                verification_code TEXT,
                raw_hash TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(account_email, raw_hash)
            );
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn import_accounts(&self, accounts: &[ImportedAccount]) -> Result<usize> {
        let mut imported = 0usize;
        for account in accounts {
            let now = Utc::now();
            let result = sqlx::query(
                r#"
                INSERT INTO accounts (
                    id, email, password, imap_host, imap_port, status,
                    lease_token, last_error, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, 'idle', '', '', ?6, ?6)
                ON CONFLICT(email) DO UPDATE SET
                    password = excluded.password,
                    imap_host = excluded.imap_host,
                    imap_port = excluded.imap_port,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&account.email)
            .bind(&account.password)
            .bind(&account.imap_host)
            .bind(i64::from(account.imap_port))
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
            if result.rows_affected() > 0 {
                imported += 1;
            }
        }
        Ok(imported)
    }

    pub async fn lease_account(&self, ttl_seconds: i64) -> Result<Option<LeasedAccount>> {
        let token = Uuid::new_v4().to_string();
        let now = Utc::now();
        let until = now + chrono::Duration::seconds(ttl_seconds.max(60));
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query(
            r#"
            SELECT id, email, password, imap_host, imap_port
            FROM accounts
            WHERE status IN ('idle', 'ready', 'error')
               OR (status = 'leased' AND lease_until IS NOT NULL AND lease_until < ?1)
            ORDER BY
              CASE status WHEN 'ready' THEN 0 WHEN 'idle' THEN 1 ELSE 2 END,
              updated_at ASC
            LIMIT 1
            "#,
        )
        .bind(now.to_rfc3339())
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };

        let id: String = row.get("id");
        sqlx::query(
            r#"
            UPDATE accounts
            SET status = 'leased', lease_token = ?1, lease_until = ?2, updated_at = ?3
            WHERE id = ?4
            "#,
        )
        .bind(&token)
        .bind(until.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(Some(LeasedAccount {
            id,
            email: row.get("email"),
            password: row.get("password"),
            imap_host: row.get("imap_host"),
            imap_port: row.get("imap_port"),
            token,
        }))
    }

    pub async fn list_accounts(&self) -> Result<Vec<AccountRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, email, imap_host, imap_port, status, last_error,
                   last_checked_at, created_at, updated_at
            FROM accounts
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(AccountRecord {
                    id: row.get("id"),
                    email: row.get("email"),
                    imap_host: row.get("imap_host"),
                    imap_port: row.get("imap_port"),
                    status: row.get("status"),
                    last_error: row.get("last_error"),
                    last_checked_at: parse_optional_datetime(row.get("last_checked_at"))?,
                    created_at: parse_datetime(row.get("created_at"))?,
                    updated_at: parse_datetime(row.get("updated_at"))?,
                })
            })
            .collect()
    }

    pub async fn get_account_secret(&self, id: &str) -> Result<Option<AccountSecret>> {
        let row = sqlx::query(
            r#"
            SELECT id, email, password, imap_host, imap_port
            FROM accounts
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| AccountSecret {
            id: row.get("id"),
            email: row.get("email"),
            password: row.get("password"),
            imap_host: row.get("imap_host"),
            imap_port: row.get("imap_port"),
        }))
    }

    pub async fn get_account_by_email(&self, email: &str) -> Result<Option<AccountSecret>> {
        let row = sqlx::query(
            r#"
            SELECT id, email, password, imap_host, imap_port
            FROM accounts
            WHERE lower(email) = lower(?1)
            "#,
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| AccountSecret {
            id: row.get("id"),
            email: row.get("email"),
            password: row.get("password"),
            imap_host: row.get("imap_host"),
            imap_port: row.get("imap_port"),
        }))
    }

    pub async fn get_account_by_token(&self, token: &str) -> Result<Option<AccountSecret>> {
        let row = sqlx::query(
            r#"
            SELECT id, email, password, imap_host, imap_port
            FROM accounts
            WHERE lease_token = ?1
            "#,
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| AccountSecret {
            id: row.get("id"),
            email: row.get("email"),
            password: row.get("password"),
            imap_host: row.get("imap_host"),
            imap_port: row.get("imap_port"),
        }))
    }

    pub async fn save_messages(&self, account_id: &str, messages: &[NewMail]) -> Result<usize> {
        let mut inserted = 0usize;
        let now = Utc::now().to_rfc3339();
        let mut tx = self.pool.begin().await?;
        for message in messages {
            let result = sqlx::query(
                r#"
                INSERT OR IGNORE INTO messages (
                    id, account_email, subject, text, html, from_addr,
                    received_at, verification_code, raw_hash, created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                "#,
            )
            .bind(&message.id)
            .bind(&message.account_email)
            .bind(&message.subject)
            .bind(&message.text)
            .bind(&message.html)
            .bind(&message.from_addr)
            .bind(message.received_at.to_rfc3339())
            .bind(&message.verification_code)
            .bind(&message.raw_hash)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
            inserted += result.rows_affected() as usize;
        }
        sqlx::query(
            r#"
            UPDATE accounts
            SET status = CASE WHEN ?2 = '' THEN 'ready' ELSE 'error' END,
                last_error = ?2,
                last_checked_at = ?3,
                updated_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind("")
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(inserted)
    }

    pub async fn mark_account_error(&self, account_id: &str, error: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            UPDATE accounts
            SET status = 'error', last_error = ?2, last_checked_at = ?3, updated_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(error.chars().take(500).collect::<String>())
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn latest_messages_for_token(&self, token: &str) -> Result<Vec<StoredMail>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id, m.account_email, m.subject, m.text, m.html, m.from_addr,
                   m.received_at, m.verification_code
            FROM messages m
            JOIN accounts a ON lower(a.email) = lower(m.account_email)
            WHERE a.lease_token = ?1
            ORDER BY m.received_at DESC, m.created_at DESC
            LIMIT 20
            "#,
        )
        .bind(token)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(row_to_mail).collect()
    }

    pub async fn message_for_token(
        &self,
        token: &str,
        message_id: &str,
    ) -> Result<Option<StoredMail>> {
        let row = sqlx::query(
            r#"
            SELECT m.id, m.account_email, m.subject, m.text, m.html, m.from_addr,
                   m.received_at, m.verification_code
            FROM messages m
            JOIN accounts a ON lower(a.email) = lower(m.account_email)
            WHERE a.lease_token = ?1 AND m.id = ?2
            LIMIT 1
            "#,
        )
        .bind(token)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(row_to_mail).transpose()
    }

    pub async fn latest_mail_for(&self, address: &str) -> Result<Option<StoredMail>> {
        let row = sqlx::query(
            r#"
            SELECT id, account_email, subject, text, html, from_addr, received_at, verification_code
            FROM messages
            WHERE lower(account_email) = lower(?1)
            ORDER BY received_at DESC, created_at DESC
            LIMIT 1
            "#,
        )
        .bind(address)
        .fetch_optional(&self.pool)
        .await?;

        row.map(row_to_mail).transpose()
    }

    pub async fn stats(&self) -> Result<Stats> {
        let total = scalar_count(&self.pool, "SELECT COUNT(*) FROM accounts").await?;
        let idle = scalar_count(
            &self.pool,
            "SELECT COUNT(*) FROM accounts WHERE status = 'idle'",
        )
        .await?;
        let leased = scalar_count(
            &self.pool,
            "SELECT COUNT(*) FROM accounts WHERE status = 'leased'",
        )
        .await?;
        let ready = scalar_count(
            &self.pool,
            "SELECT COUNT(*) FROM accounts WHERE status = 'ready'",
        )
        .await?;
        let error = scalar_count(
            &self.pool,
            "SELECT COUNT(*) FROM accounts WHERE status = 'error'",
        )
        .await?;
        let messages = scalar_count(&self.pool, "SELECT COUNT(*) FROM messages").await?;
        Ok(Stats {
            total,
            idle,
            leased,
            ready,
            error,
            messages,
        })
    }

    pub async fn delete_account(&self, id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let email = sqlx::query("SELECT email FROM accounts WHERE id = ?1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .map(|row| row.get::<String, _>("email"));
        let Some(email) = email else {
            tx.commit().await?;
            return Ok(false);
        };
        sqlx::query("DELETE FROM messages WHERE lower(account_email) = lower(?1)")
            .bind(&email)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM accounts WHERE id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}

async fn scalar_count(pool: &SqlitePool, sql: &str) -> Result<i64> {
    let row = sqlx::query(sql).fetch_one(pool).await?;
    Ok(row.get::<i64, _>(0))
}

fn row_to_mail(row: sqlx::sqlite::SqliteRow) -> Result<StoredMail> {
    Ok(StoredMail {
        id: row.get("id"),
        account_email: row.get("account_email"),
        subject: row.get("subject"),
        text: row.get("text"),
        html: row.get("html"),
        from_addr: row.get("from_addr"),
        received_at: parse_datetime(row.get("received_at"))?,
        verification_code: row.get("verification_code"),
    })
}

fn parse_datetime(raw: String) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(&raw)?.with_timezone(&Utc))
}

fn parse_optional_datetime(raw: Option<String>) -> Result<Option<DateTime<Utc>>> {
    raw.map(parse_datetime).transpose()
}

#[cfg(test)]
mod tests {
    use super::Store;
    use crate::importer::ImportedAccount;

    async fn memory_store() -> Store {
        Store::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn imports_and_leases_distinct_accounts() {
        let store = memory_store().await;
        store
            .import_accounts(&[
                ImportedAccount {
                    email: "a@outlook.com".to_string(),
                    password: "a-pass".to_string(),
                    imap_host: "outlook.office365.com".to_string(),
                    imap_port: 993,
                },
                ImportedAccount {
                    email: "b@outlook.com".to_string(),
                    password: "b-pass".to_string(),
                    imap_host: "outlook.office365.com".to_string(),
                    imap_port: 993,
                },
            ])
            .await
            .unwrap();

        let first = store.lease_account(300).await.unwrap().unwrap();
        let second = store.lease_account(300).await.unwrap().unwrap();

        assert_ne!(first.email, second.email);
        assert!(!first.token.is_empty());
        assert!(!second.token.is_empty());
        assert!(store.lease_account(300).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn stores_messages_and_filters_by_lease_token() {
        let store = memory_store().await;
        store
            .import_accounts(&[ImportedAccount {
                email: "a@outlook.com".to_string(),
                password: "a-pass".to_string(),
                imap_host: "outlook.office365.com".to_string(),
                imap_port: 993,
            }])
            .await
            .unwrap();

        let lease = store.lease_account(300).await.unwrap().unwrap();
        store
            .save_messages(
                &lease.id,
                &[super::NewMail {
                    id: "msg-1".to_string(),
                    account_email: lease.email.clone(),
                    subject: "Your code 123456".to_string(),
                    text: "123456".to_string(),
                    html: String::new(),
                    from_addr: "noreply@tm.openai.com".to_string(),
                    received_at: chrono::Utc::now(),
                    verification_code: Some("123456".to_string()),
                    raw_hash: "hash-1".to_string(),
                }],
            )
            .await
            .unwrap();

        let messages = store.latest_messages_for_token(&lease.token).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].verification_code.as_deref(), Some("123456"));
        assert!(
            store
                .latest_messages_for_token("wrong")
                .await
                .unwrap()
                .is_empty()
        );
    }
}
