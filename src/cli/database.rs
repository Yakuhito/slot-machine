use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
use std::time::Duration;

pub struct Database {
    pool: Pool<Sqlite>,
}

impl Database {
    // Initializes a new database and creates the table if it doesn't exist
    pub async fn new() -> Result<Self, sqlx::Error> {
        let pool = SqlitePoolOptions::new()
            .connect_timeout(Duration::from_secs(5))
            .connect("sqlite://data.db")
            .await?;

        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS key_value_store (
                id INTEGER PRIMARY KEY,
                key TEXT UNIQUE NOT NULL,
                value TEXT NOT NULL
            )
            ",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    // Saves a key-value pair into the database
    pub async fn save_key_value(&self, key: &str, value: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "
            INSERT INTO key_value_store (key, value) 
            VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            ",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // Retrieves a value from the database by key
    pub async fn get_value_by_key(&self, key: &str) -> Result<Option<String>, sqlx::Error> {
        let row = sqlx::query(
            "
            SELECT value FROM key_value_store WHERE key = ?1
            ",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.get(0)))
    }
}
