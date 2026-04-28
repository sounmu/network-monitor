//! Low-level SQL for the `refresh_tokens` table.
//!
//! Hash-based lookups only — the plaintext token never leaves the SHA-256
//! boundary inside `services::refresh_token`. See `migrations/011_refresh_tokens.sql`
//! for the full schema rationale.

use chrono::{DateTime, Utc};

use crate::db::DbPool;

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct RefreshTokenRow {
    pub id: i64,
    pub user_id: i32,
    pub family_id: Vec<u8>,
    pub parent_id: Option<i64>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

// Schema mirrors the pre-migration Postgres table column list with
// `BYTEA → BLOB` for the hash/family columns and `TIMESTAMPTZ → INTEGER
// epoch`. `&[u8]` binds straight into the BLOB columns. sqlx-sqlite
// binds `DateTime<Utc>` as TEXT ISO-8601 by default, which an INTEGER
// column rejects — `.timestamp()` is applied on every write path so
// the integer lands in the column directly. Decoding INTEGER back into
// `DateTime<Utc>` works without a shim.

/// Insert a new refresh token row and return the generated id.
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &DbPool,
    user_id: i32,
    token_hash: &[u8],
    family_id: &[u8],
    parent_id: Option<i64>,
    expires_at: DateTime<Utc>,
    user_agent: Option<&str>,
    ip: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let (id,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO refresh_tokens
            (user_id, token_hash, family_id, parent_id, expires_at, user_agent, ip)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(family_id)
    .bind(parent_id)
    .bind(expires_at.timestamp())
    .bind(user_agent)
    .bind(ip)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Look up a token row by its stored hash. Returns `None` if not present —
/// callers must not distinguish "not found" from "revoked" in the response
/// path (no enumeration oracle).
pub async fn find_by_hash(
    pool: &DbPool,
    token_hash: &[u8],
) -> Result<Option<RefreshTokenRow>, sqlx::Error> {
    sqlx::query_as::<_, RefreshTokenRow>(
        r#"
        SELECT id, user_id, family_id, parent_id, issued_at, expires_at, revoked_at
        FROM refresh_tokens
        WHERE token_hash = ?1
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

/// Mark a single row as revoked. Idempotent — already-revoked rows stay
/// pointing at their original revocation instant.
///
/// Kept alongside `revoke_all_for_user` for future single-session admin
/// tooling; currently only the user-level revoke is wired into handlers.
#[allow(dead_code)]
pub async fn revoke_by_id(pool: &DbPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = strftime('%s','now') \
         WHERE id = ?1 AND revoked_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Revoke every live row in the given family. Used when reuse is detected.
pub async fn revoke_family(pool: &DbPool, family_id: &[u8]) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = strftime('%s','now') \
         WHERE family_id = ?1 AND revoked_at IS NULL",
    )
    .bind(family_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Revoke every live row for a user. Used by logout and admin kill-switch.
pub async fn revoke_all_for_user(pool: &DbPool, user_id: i32) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = strftime('%s','now') \
         WHERE user_id = ?1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Delete rows that expired more than the grace window ago. Called
/// periodically from a background task so the table does not grow
/// without bound.
pub async fn delete_expired(pool: &DbPool) -> Result<u64, sqlx::Error> {
    let res =
        sqlx::query("DELETE FROM refresh_tokens WHERE expires_at < strftime('%s','now') - 7*86400")
            .execute(pool)
            .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> DbPool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Memory);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_find_revoke_roundtrip() {
        let pool = fresh_pool().await;
        let hash = [0xAAu8; 32];
        let family = [0xBBu8; 16];
        let expires = Utc::now() + chrono::Duration::days(14);

        let id = insert(
            &pool,
            1,
            &hash,
            &family,
            None,
            expires,
            Some("ua"),
            Some("1.2.3.4"),
        )
        .await
        .unwrap();
        assert!(id >= 1);

        let row = find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert_eq!(row.user_id, 1);
        assert_eq!(row.family_id, family.to_vec());
        assert!(row.revoked_at.is_none());

        revoke_by_id(&pool, id).await.unwrap();
        let after = find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert!(after.revoked_at.is_some());
    }

    #[tokio::test]
    async fn family_revocation_hits_every_sibling() {
        let pool = fresh_pool().await;
        let family = [0xCCu8; 16];
        let expires = Utc::now() + chrono::Duration::days(14);

        for i in 0..3u8 {
            let mut hash = [0u8; 32];
            hash[0] = i;
            insert(&pool, 42, &hash, &family, None, expires, None, None)
                .await
                .unwrap();
        }
        // Another family for the same user — must NOT be touched.
        let other_family = [0xDDu8; 16];
        insert(
            &pool,
            42,
            &[0x11u8; 32],
            &other_family,
            None,
            expires,
            None,
            None,
        )
        .await
        .unwrap();

        let revoked = revoke_family(&pool, &family).await.unwrap();
        assert_eq!(revoked, 3);

        let total_revoked = revoke_family(&pool, &family).await.unwrap();
        assert_eq!(total_revoked, 0, "already-revoked rows are skipped");

        let other_still_live = find_by_hash(&pool, &[0x11u8; 32]).await.unwrap().unwrap();
        assert!(other_still_live.revoked_at.is_none());
    }
}
