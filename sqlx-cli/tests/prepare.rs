use assert_cmd::cargo_bin_cmd;
use serde::Deserialize;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Executor, SqliteConnection};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tempfile::TempDir;

#[derive(Deserialize)]
struct TestManifest {
    packages: BTreeMap<String, TestPackageQueries>,
}

#[derive(Deserialize)]
struct TestPackageQueries {
    queries: Vec<String>,
}

fn workspace_root() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .display()
        .to_string()
}

fn write_test_project(tempdir: &TempDir) {
    write_query_source(tempdir, "select id from users");
}

fn write_query_source(tempdir: &TempDir, query: &str) {
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
        format!("fn main() {{\n    let _ = sqlx::query!(\"{query}\");\n}}\n"),
    )
    .unwrap();
}

fn sqlx_query_files(tempdir: &TempDir) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(tempdir.path().join(".sqlx"))
        .unwrap()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.file_name()?
                .to_string_lossy()
                .starts_with("query-")
                && path.extension().is_some_and(|ext| ext == "json"))
            .then_some(path)
        })
        .collect();
    files.sort();
    files
}

fn read_prepare_manifest(tempdir: &TempDir) -> TestManifest {
    serde_json::from_slice(
        &fs::read(tempdir.path().join("target/sqlx-prepare-manifest.json")).unwrap(),
    )
    .unwrap()
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
async fn detect_query_changes_prunes_stale_query_file() {
    let tempdir = TempDir::new().unwrap();
    write_test_project(&tempdir);

    let db_path = tempdir.path().join("prepare-prune.db");
    let database_url = format!("sqlite://{}", db_path.display());
    setup_sqlite_db(&database_url).await;

    cargo_bin_cmd!("cargo-sqlx")
        .current_dir(tempdir.path())
        .args([
            "sqlx",
            "prepare",
            "--database-url",
            &database_url,
            "--detect-query-changes",
        ])
        .assert()
        .success();

    let old_files = sqlx_query_files(&tempdir);
    assert_eq!(old_files.len(), 1);
    let old_manifest = read_prepare_manifest(&tempdir);

    write_query_source(&tempdir, "select id, id as id2 from users");

    let assert = cargo_bin_cmd!("cargo-sqlx")
        .current_dir(tempdir.path())
        .args([
            "sqlx",
            "prepare",
            "--database-url",
            &database_url,
            "--detect-query-changes",
            "--verbose",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let new_manifest = read_prepare_manifest(&tempdir);
    assert_ne!(
        old_manifest.packages.values().next().unwrap().queries,
        new_manifest.packages.values().next().unwrap().queries,
    );
    assert!(stdout.contains("stale query files pruned: 1"), "stdout was:\n{stdout}");

    let new_files = sqlx_query_files(&tempdir);
    assert_eq!(new_files.len(), 1);
    assert_ne!(old_files[0], new_files[0]);
    assert!(!old_files[0].exists());
}
