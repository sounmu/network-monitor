use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

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

pub async fn init_table(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id            SERIAL PRIMARY KEY,
            username      TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role          TEXT NOT NULL DEFAULT 'admin',
            created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn count_users(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*)::BIGINT FROM users")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

pub async fn find_by_username(
    pool: &PgPool,
    username: &str,
) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE username = $1")
        .bind(username)
        .fetch_optional(pool)
        .await
}

pub async fn find_by_id(pool: &PgPool, user_id: i32) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

pub async fn create_user(
    pool: &PgPool,
    username: &str,
    password_hash: &str,
    role: &str,
) -> Result<UserRow, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        r#"
        INSERT INTO users (username, password_hash, role)
        VALUES ($1, $2, $3)
        RETURNING *
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .bind(role)
    .fetch_one(pool)
    .await
}

pub async fn update_password(
    pool: &PgPool,
    user_id: i32,
    new_password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET password_hash = $1, password_changed_at = NOW(), updated_at = NOW() WHERE id = $2",
    )
    .bind(new_password_hash)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Load password_changed_at timestamps for all users (startup cache population).
pub async fn load_password_changed_at(
    pool: &PgPool,
) -> Result<std::collections::HashMap<i32, i64>, sqlx::Error> {
    let rows: Vec<(i32, DateTime<Utc>)> =
        sqlx::query_as("SELECT id, password_changed_at FROM users")
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, ts)| (id, ts.timestamp()))
        .collect())
}

/// Stamp `tokens_revoked_at = NOW()` for a user. Called on logout and admin
/// session-kill. Any JWT whose `iat` predates this row is invalidated.
pub async fn revoke_user_tokens(pool: &PgPool, user_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET tokens_revoked_at = NOW(), updated_at = NOW() WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Load tokens_revoked_at timestamps for users who have one set (startup cache).
/// Users who have never had a revocation return no row, so the map is sparse.
pub async fn load_tokens_revoked_at(
    pool: &PgPool,
) -> Result<std::collections::HashMap<i32, i64>, sqlx::Error> {
    let rows: Vec<(i32, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, tokens_revoked_at FROM users WHERE tokens_revoked_at IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, ts)| (id, ts.timestamp()))
        .collect())
}
