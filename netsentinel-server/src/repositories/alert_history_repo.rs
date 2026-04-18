use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, QueryBuilder};

/// Row from the `alert_history` table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AlertHistoryRow {
    pub id: i64,
    pub host_key: String,
    pub alert_type: String,
    pub message: String,
    pub created_at: DateTime<Utc>,
}

/// Create the alert_history table
pub async fn init_table(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS alert_history (
            id           BIGSERIAL PRIMARY KEY,
            host_key     VARCHAR(255) NOT NULL,
            alert_type   VARCHAR(100) NOT NULL,
            message      TEXT NOT NULL,
            created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_alert_history_host_time ON alert_history (host_key, created_at DESC)",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Insert an alert history record
pub async fn insert_alert(
    pool: &PgPool,
    host_key: &str,
    alert_type: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO alert_history (host_key, alert_type, message) VALUES ($1, $2, $3)")
        .bind(host_key)
        .bind(alert_type)
        .bind(message)
        .execute(pool)
        .await?;
    Ok(())
}

/// Query parameters for alert history listing.
#[derive(Debug, Default, Deserialize)]
pub struct AlertHistoryQuery {
    pub host_key: Option<String>,
    /// Filter by a specific alert_type (e.g. `cpu_overload`).
    #[serde(rename = "type")]
    pub alert_type: Option<String>,
    /// Inclusive lower bound on `created_at`, RFC3339.
    pub from: Option<DateTime<Utc>>,
    /// Exclusive upper bound on `created_at`, RFC3339.
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Page of alert history rows together with the total number of rows that
/// match the same filter (used to drive client pagination).
#[derive(Debug, Serialize)]
pub struct AlertHistoryPage {
    pub rows: Vec<AlertHistoryRow>,
    pub total: i64,
}

/// Fetch alert history with optional filters + pagination and return the
/// matching total in the same round-trip so clients can render "showing X
/// of Y" labels without an extra COUNT request.
pub async fn get_alert_history_page(
    pool: &PgPool,
    query: &AlertHistoryQuery,
) -> Result<AlertHistoryPage, sqlx::Error> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).clamp(0, 10_000);

    let mut list = QueryBuilder::<Postgres>::new("SELECT * FROM alert_history WHERE 1=1");
    let mut count =
        QueryBuilder::<Postgres>::new("SELECT COUNT(*)::BIGINT FROM alert_history WHERE 1=1");

    if let Some(ref hk) = query.host_key {
        list.push(" AND host_key = ").push_bind(hk.clone());
        count.push(" AND host_key = ").push_bind(hk.clone());
    }
    if let Some(ref at) = query.alert_type {
        list.push(" AND alert_type = ").push_bind(at.clone());
        count.push(" AND alert_type = ").push_bind(at.clone());
    }
    if let Some(from) = query.from {
        list.push(" AND created_at >= ").push_bind(from);
        count.push(" AND created_at >= ").push_bind(from);
    }
    if let Some(to) = query.to {
        list.push(" AND created_at < ").push_bind(to);
        count.push(" AND created_at < ").push_bind(to);
    }

    list.push(" ORDER BY created_at DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    let rows = list
        .build_query_as::<AlertHistoryRow>()
        .fetch_all(pool)
        .await?;
    let total: (i64,) = count.build_query_as().fetch_one(pool).await?;

    Ok(AlertHistoryPage {
        rows,
        total: total.0,
    })
}

/// Compute currently-firing alerts by picking the latest event per
/// (host_key, base_alert_kind) and returning only those that end in
/// `_overload` / `_down`.
pub async fn get_active_alerts(pool: &PgPool) -> Result<Vec<AlertHistoryRow>, sqlx::Error> {
    sqlx::query_as::<_, AlertHistoryRow>(
        r#"
        WITH paired AS (
            SELECT
                id,
                host_key,
                alert_type,
                message,
                created_at,
                regexp_replace(alert_type, '_(overload|recovery|down)$', '') AS base_kind
            FROM alert_history
            WHERE created_at > NOW() - INTERVAL '14 days'
        ),
        latest AS (
            SELECT DISTINCT ON (host_key, base_kind)
                id, host_key, alert_type, message, created_at
            FROM paired
            ORDER BY host_key, base_kind, created_at DESC
        )
        SELECT id, host_key, alert_type, message, created_at
        FROM latest
        WHERE alert_type LIKE '%_overload' OR alert_type LIKE '%_down'
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await
}
