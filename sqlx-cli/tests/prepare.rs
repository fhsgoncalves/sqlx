use assert_cmd::cargo_bin_cmd;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Executor, SqliteConnection};
use std::fs;
use std::path::Path;
use std::str::FromStr;
use tempfile::TempDir;

fn workspace_root() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .display()
        .to_string()
}

fn write_test_project(tempdir: &TempDir) {
    let root = workspace_root();

    fs::write(
        tempdir.path().join("Cargo.toml"),
        format!(
            r#"[package]
name = "prepare-test"
version = "0.1.0"
edition = "2021"

[dependencies]
sqlx = {{ path = "{root}", default-features = false, features = ["sqlite", "macros", "runtime-tokio"] }}
tokio = {{ version = "1", features = ["macros", "rt-multi-thread"] }}
"#
        ),
    )
    .unwrap();

    fs::create_dir_all(tempdir.path().join("src")).unwrap();
    fs::write(
        tempdir.path().join("src/main.rs"),
        r#"fn main() {
    let _ = sqlx::query!("select id from users");
}
"#,
    )
    .unwrap();
}

async fn setup_sqlite_db(database_url: &str) {
    let options = SqliteConnectOptions::from_str(database_url)
        .unwrap()
        .create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
    conn.execute("create table users (id integer not null);")
        .await
        .unwrap();
    conn.close().await.unwrap();
}

#[tokio::test]
async fn prepare_verbose_reports_selective_path() {
    let tempdir = TempDir::new().unwrap();
    write_test_project(&tempdir);

    let db_path = tempdir.path().join("prepare.db");
    let database_url = format!("sqlite://{}", db_path.display());
    setup_sqlite_db(&database_url).await;

    cargo_bin_cmd!("cargo-sqlx")
        .current_dir(tempdir.path())
        .args([
            "sqlx",
            "prepare",
            "--database-url",
            &database_url,
            "--experimental-schema-change-detection",
            "--verbose",
        ])
        .assert()
        .success();

    let assert = cargo_bin_cmd!("cargo-sqlx")
        .current_dir(tempdir.path())
        .args([
            "sqlx",
            "prepare",
            "--database-url",
            &database_url,
            "--experimental-schema-change-detection",
            "--verbose",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("prepare verbose:"));
    assert!(stdout.contains("path: selective prepare"));
    assert!(stdout.contains("experimental-schema-change-detection: true"));
    assert!(stdout.contains("packages selected: 0"));
    assert!(stdout.contains("no packages selected"));
    assert!(stdout.contains("query data unchanged; skipping recompilation"));
}
