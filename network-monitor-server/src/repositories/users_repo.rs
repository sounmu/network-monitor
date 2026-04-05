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
    sqlx::query("UPDATE users SET password_hash = $1, updated_at = NOW() WHERE id = $2")
        .bind(new_password_hash)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
