use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::DbPool;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserRow {
    pub id: i32,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Public user info (no password hash)
#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: i32,
    pub username: String,
    pub role: String,
}

impl From<UserRow> for UserInfo {
    fn from(row: UserRow) -> Self {
        Self {
            id: row.id,
            username: row.username,
            role: row.role,
        }
    }
}

pub async fn count_users(pool: &DbPool) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

pub async fn find_by_username(
    pool: &DbPool,
    username: &str,
) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, role, created_at, updated_at FROM users WHERE username = ?1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await
}

pub async fn find_by_id(pool: &DbPool, user_id: i32) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, role, created_at, updated_at FROM users WHERE id = ?1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Create a user. Accepts both a pool reference and a transaction as the
/// executor, so callers can run it inside a larger atomic change.
pub async fn create_user<'e, E: sqlx::Executor<'e, Database = sqlx::Sqlite>>(
    executor: E,
    username: &str,
    password_hash: &str,
    role: &str,
) -> Result<UserRow, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        r#"
        INSERT INTO users (username, password_hash, role)
        VALUES (?1, ?2, ?3)
        RETURNING id, username, password_hash, role, created_at, updated_at
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .bind(role)
    .fetch_one(executor)
    .await
}

pub async fn update_password(
    pool: &DbPool,
    user_id: i32,
    new_password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET password_hash = ?1, \
         password_changed_at = strftime('%s','now'), \
         updated_at = strftime('%s','now') WHERE id = ?2",
    )
    .bind(new_password_hash)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Load password_changed_at timestamps for all users (startup cache population).
pub async fn load_password_changed_at(
    pool: &DbPool,
) -> Result<std::collections::HashMap<i32, i64>, sqlx::Error> {
    let rows: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT id, password_changed_at FROM users WHERE password_changed_at IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Stamp `tokens_revoked_at = now` for a user. Called on logout and admin
/// session-kill. Any JWT whose `iat` predates this row is invalidated.
pub async fn revoke_user_tokens(pool: &DbPool, user_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET tokens_revoked_at = strftime('%s','now'), \
         updated_at = strftime('%s','now') WHERE id = ?1",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Load tokens_revoked_at timestamps for users who have one set (startup cache).
/// Users who have never had a revocation return no row, so the map is sparse.
pub async fn load_tokens_revoked_at(
    pool: &DbPool,
) -> Result<std::collections::HashMap<i32, i64>, sqlx::Error> {
    let rows: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT id, tokens_revoked_at FROM users WHERE tokens_revoked_at IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
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
    async fn create_then_find_roundtrips_user_row() {
        let pool = fresh_pool().await;

        assert_eq!(count_users(&pool).await.unwrap(), 0);

        let user = create_user(&pool, "alice", "hash-placeholder", "admin")
            .await
            .unwrap();
        assert_eq!(user.username, "alice");
        assert_eq!(user.role, "admin");
        assert!(user.id >= 1);

        assert_eq!(count_users(&pool).await.unwrap(), 1);

        let by_name = find_by_username(&pool, "alice").await.unwrap().unwrap();
        assert_eq!(by_name.id, user.id);
        assert_eq!(by_name.password_hash, "hash-placeholder");

        let by_id = find_by_id(&pool, user.id).await.unwrap().unwrap();
        assert_eq!(by_id.username, "alice");

        // Uniqueness on `username` is enforced by the schema.
        let duplicate = create_user(&pool, "alice", "other", "viewer").await;
        assert!(
            duplicate.is_err(),
            "expected UNIQUE violation on duplicate username"
        );
    }

    #[tokio::test]
    async fn revocation_timestamps_load_back_as_maps() {
        let pool = fresh_pool().await;
        let a = create_user(&pool, "a", "h", "admin").await.unwrap();
        let b = create_user(&pool, "b", "h", "viewer").await.unwrap();

        // No revocations yet — both maps should be empty.
        assert!(load_tokens_revoked_at(&pool).await.unwrap().is_empty());
        assert!(load_password_changed_at(&pool).await.unwrap().is_empty());

        revoke_user_tokens(&pool, a.id).await.unwrap();
        update_password(&pool, b.id, "new-hash").await.unwrap();

        let revoked = load_tokens_revoked_at(&pool).await.unwrap();
        assert_eq!(revoked.len(), 1);
        assert!(revoked.contains_key(&a.id));

        let changed = load_password_changed_at(&pool).await.unwrap();
        assert_eq!(changed.len(), 1);
        assert!(changed.contains_key(&b.id));
    }
}
