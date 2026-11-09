// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::atomic::{AtomicU64, Ordering};

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::Mutex;

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Serializes `sqlx::migrate!` across `#[tokio::test]` runtimes.
///
/// `0001_init.sql` creates one `positions` partition per month from 2012 through
/// 2035, and each partition keeps several locks until that transaction commits.
/// The lock table is cluster-wide (`max_locks_per_transaction`, Postgres SQLSTATE
/// 53200). Parallel migrations overflow it. The lock is released before the test
/// body runs, so queries still overlap. The server process migrates once at
/// startup and does not take this lock.
static MIGRATION: Mutex<()> = Mutex::const_new(());

pub(crate) struct TestDb {
    pub pool: PgPool,
    #[allow(dead_code)]
    pub url: String,
    name: String,
    admin_url: String,
}

impl TestDb {
    pub(crate) async fn create(prefix: &str) -> Option<Self> {
        let admin_url = std::env::var("DATABASE_URL").ok()?;
        if admin_url.is_empty() {
            assert!(
                std::env::var_os("CI").is_none(),
                "DATABASE_URL is required in CI"
            );
            eprintln!("skipping database test; DATABASE_URL is unset");
            return None;
        }
        let name = sanitize(prefix);
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("admin connection");
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .expect("create database");
        admin.close().await;
        let url = with_database(&admin_url, &name);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("test database");
        Some(Self {
            pool,
            url,
            name,
            admin_url,
        })
    }

    pub(crate) async fn migrated(prefix: &str) -> Option<Self> {
        let db = Self::create(prefix).await?;
        let guard = MIGRATION.lock().await;
        sqlx::migrate!("./migrations")
            .run(&db.pool)
            .await
            .expect("migrate");
        drop(guard);
        Some(db)
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let name = self.name.clone();
        let admin_url = self.admin_url.clone();
        // Detach so dropping the pool can finish on the test runtime. Joining here deadlocks
        // sqlx, which waits for that runtime while `Drop` is running on it.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                if let Ok(admin) = PgPool::connect(&admin_url).await {
                    let statement = format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)");
                    let _ = sqlx::query(&statement).execute(&admin).await;
                    admin.close().await;
                }
            });
        });
    }
}

fn sanitize(prefix: &str) -> String {
    let mut name = String::from("tl_");
    for ch in prefix.chars() {
        if ch.is_ascii_alphanumeric() {
            name.push(ch.to_ascii_lowercase());
        } else {
            name.push('_');
        }
    }
    name.push('_');
    name.push_str(&std::process::id().to_string());
    name.push('_');
    name.push_str(&COUNTER.fetch_add(1, Ordering::Relaxed).to_string());
    name
}

fn with_database(url: &str, name: &str) -> String {
    let mut parsed = url::Url::parse(url).expect("DATABASE_URL");
    parsed.set_path(&format!("/{name}"));
    parsed.to_string()
}
