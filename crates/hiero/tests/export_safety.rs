//! `hiero export` destination safety (finding A1): the export is a read-only
//! diagnostic that serializes memory content to a NEW file. It must never be
//! able to destroy the data root it reads from — not through the database
//! path, not through an alias to it, not through any other runtime file — and
//! it must never silently replace an existing file. The read side must also
//! observe one committed snapshot, so a concurrent daemon write can never
//! split a foreign-key pair across two exported tables.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hieronymus::data_root::HieronymusConfig;

/// Open a data root with a live, migrated database, exactly as the CLI would
/// before an export runs.
fn open_root() -> (tempfile::TempDir, HieronymusConfig) {
    let root = native_tempdir();
    let config = HieronymusConfig::new(root.path());
    let application = hiero::application::Application::open(&config).unwrap();
    // Keep the application only for its side effect (a migrated database);
    // holding it open would keep a WAL reader around for no reason.
    drop(application);
    (root, config)
}

/// Assert `run` refused `destination` and left it byte-identical (or absent).
fn assert_refused_without_touching(config: &HieronymusConfig, destination: &Path) {
    let before = std::fs::read(destination).ok();
    let error = hiero::export::run(config, destination)
        .err()
        .unwrap_or_else(|| panic!("export must refuse {}", destination.display()));
    assert_eq!(
        std::fs::read(destination).ok(),
        before,
        "{} changed despite the refusal: {error}",
        destination.display()
    );
}

#[test]
fn export_refuses_authoritative_database() {
    let (_root, config) = open_root();
    let before = std::fs::read(config.database_path()).unwrap();
    assert!(hiero::export::run(&config, &config.database_path()).is_err());
    assert_eq!(std::fs::read(config.database_path()).unwrap(), before);
}

#[test]
fn export_refuses_runtime_files_and_managed_trees() {
    let (root, config) = open_root();
    // Materialize the runtime files an installed root carries, so the guard is
    // exercised against real inodes and not only against path arithmetic.
    for path in [
        config.provider_config_path(),
        config.dream_config_path(),
        config.ingest_config_path(),
        config.release_config_path(),
        config.daemon_discovery_path(),
        config.daemon_token_path(),
        hieronymus::semantic_arming::semantic_config_path(&config),
        config
            .data_root()
            .join(hieronymus::ownership::OWNER_LOCK_FILE),
        config.data_root().join(hieronymus::upgrade::JOURNAL_FILE),
        config.dream_autostart_path(),
        hieronymus::dream_locks::dream_cycle_paths(&config).lock_file,
    ] {
        std::fs::write(&path, b"runtime\n").unwrap();
    }
    for tree in [
        config.semantic_root(),
        config.backups_root(),
        config.agent_plugins_root(),
        config.data_root().join(hieronymus::rag::RAG_NORMALIZED_DIR),
    ] {
        std::fs::create_dir_all(&tree).unwrap();
    }

    let database = config.database_path();
    let sidecar = |suffix: &str| {
        let mut name = database.clone().into_os_string();
        name.push(suffix);
        PathBuf::from(name)
    };
    let refused: Vec<PathBuf> = vec![
        database.clone(),
        sidecar("-wal"),
        sidecar("-shm"),
        sidecar("-journal"),
        config.provider_config_path(),
        config.dream_config_path(),
        config.ingest_config_path(),
        config.release_config_path(),
        config.llm_cache_path(),
        config.daemon_discovery_path(),
        config.daemon_token_path(),
        hieronymus::semantic_arming::semantic_config_path(&config),
        config
            .data_root()
            .join(hieronymus::ownership::OWNER_LOCK_FILE),
        config.data_root().join(hieronymus::upgrade::JOURNAL_FILE),
        config.dream_autostart_path(),
        hieronymus::dream_locks::dream_cycle_paths(&config).lock_file,
        hieronymus::dream_locks::dream_cycle_paths(&config).state_json,
        config.semantic_root().join("index").join("data.lance"),
        config.backups_root().join("2026-09-06").join("db.sqlite"),
        config.agent_plugins_root().join("claude").join("hook.json"),
        config
            .data_root()
            .join(hieronymus::rag::RAG_NORMALIZED_DIR)
            .join("chapter-01.txt"),
    ];
    for destination in &refused {
        assert_refused_without_touching(&config, destination);
    }

    // The guard is targeted, not a blanket ban on the data root: an ordinary
    // new file beside the runtime state is still a legal destination.
    let allowed = root.path().join("exports").join("memory.json");
    hiero::export::run(&config, &allowed).unwrap();
    assert!(allowed.is_file());
}

#[test]
#[cfg(unix)]
fn export_refuses_a_symlink_alias_to_the_database() {
    let (root, config) = open_root();
    let alias = root.path().join("alias.json");
    std::os::unix::fs::symlink(config.database_path(), &alias).unwrap();

    let before = std::fs::read(config.database_path()).unwrap();
    assert!(hiero::export::run(&config, &alias).is_err());
    assert_eq!(std::fs::read(config.database_path()).unwrap(), before);
    assert!(
        std::fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the alias itself must survive untouched"
    );
}

#[test]
#[cfg(unix)]
fn export_refuses_a_symlinked_parent_directory() {
    let (root, config) = open_root();
    let link = root.path().join("root-link");
    std::os::unix::fs::symlink(config.data_root(), &link).unwrap();

    let before = std::fs::read(config.database_path()).unwrap();
    assert!(hiero::export::run(&config, &link.join("hieronymus.sqlite")).is_err());
    assert_eq!(std::fs::read(config.database_path()).unwrap(), before);
}

#[test]
fn export_refuses_a_hard_link_alias_to_the_database() {
    let (root, config) = open_root();
    let alias = root.path().join("hardlink.json");
    std::fs::hard_link(config.database_path(), &alias).unwrap();

    let before = std::fs::read(config.database_path()).unwrap();
    assert!(hiero::export::run(&config, &alias).is_err());
    assert_eq!(std::fs::read(config.database_path()).unwrap(), before);
    // The alias shares the database's inode: a write through it would have
    // destroyed the database as surely as writing the database path.
    assert_eq!(std::fs::read(&alias).unwrap(), before);
}

#[test]
fn export_never_clobbers_an_existing_destination() {
    let (root, config) = open_root();
    let destination = root.path().join("exports").join("memory.json");
    hiero::export::run(&config, &destination).unwrap();
    let first = std::fs::read(&destination).unwrap();

    // A second export to the same path is refused: overwriting is a separate,
    // deliberate act (remove or rename the previous export first).
    let error = hiero::export::run(&config, &destination).unwrap_err();
    assert!(
        error.to_string().contains("already exists"),
        "the refusal must name the cause: {error}"
    );
    // Never truncated, never half-written: the previous export is intact.
    assert_eq!(std::fs::read(&destination).unwrap(), first);

    // The same holds for a file this export never produced.
    let foreign = root.path().join("notes.txt");
    std::fs::write(&foreign, b"important user notes\n").unwrap();
    assert!(hiero::export::run(&config, &foreign).is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), b"important user notes\n");
}

#[test]
fn an_explicit_overwrite_replaces_an_existing_export() {
    let (root, config) = open_root();
    let destination = root.path().join("exports").join("memory.json");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::write(&destination, b"stale export\n").unwrap();
    let database_before = std::fs::read(config.database_path()).unwrap();

    let report = hiero::export::run_overwriting(&config, &destination).unwrap();
    assert_eq!(report.output, destination);
    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&destination).unwrap()).unwrap();
    assert_eq!(document["format"], hiero::export::EXPORT_FORMAT);

    // The replacement is a rename, not a truncation: no temporary file is left
    // beside the destination.
    let leftovers: Vec<String> = std::fs::read_dir(root.path().join("exports"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["memory.json".to_string()]);

    // Overwriting a destination never licenses touching the source.
    assert_eq!(
        std::fs::read(config.database_path()).unwrap(),
        database_before
    );

    // A fresh destination works through the overwrite entry point too.
    let fresh = root.path().join("exports").join("nested").join("copy.json");
    hiero::export::run_overwriting(&config, &fresh).unwrap();
    assert_eq!(
        std::fs::read_to_string(&fresh).unwrap(),
        std::fs::read_to_string(&destination).unwrap()
    );
}

#[test]
fn an_explicit_overwrite_still_refuses_protected_destinations() {
    let (root, config) = open_root();
    #[cfg(unix)]
    let symlink_alias = {
        let alias = root.path().join("alias.json");
        std::os::unix::fs::symlink(config.database_path(), &alias).unwrap();
        alias
    };
    let hardlink_alias = root.path().join("hardlink.json");
    std::fs::hard_link(config.database_path(), &hardlink_alias).unwrap();
    std::fs::write(config.daemon_token_path(), b"secret\n").unwrap();

    let before = std::fs::read(config.database_path()).unwrap();
    // `--force` is permission to replace the user's own export. It is never
    // permission to replace the user's memory or the installation's state.
    for destination in [
        config.database_path(),
        #[cfg(unix)]
        symlink_alias.clone(),
        hardlink_alias.clone(),
        config.daemon_token_path(),
        config.data_root().join("backups").join("db.sqlite"),
    ] {
        assert!(
            hiero::export::run_overwriting(&config, &destination).is_err(),
            "--force must not reach {}",
            destination.display()
        );
    }
    assert_eq!(std::fs::read(config.database_path()).unwrap(), before);
    assert_eq!(std::fs::read(&hardlink_alias).unwrap(), before);
    assert_eq!(
        std::fs::read_to_string(config.daemon_token_path()).unwrap(),
        "secret\n"
    );
}

#[test]
fn a_failed_overwrite_leaves_the_previous_file_intact() {
    let (root, config) = open_root();
    // The destination is a non-empty directory: the publishing rename cannot
    // succeed, so the overwrite path must fail without destroying anything.
    let destination = root.path().join("exports").join("memory.json");
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("keep.txt"), b"kept\n").unwrap();

    assert!(hiero::export::run_overwriting(&config, &destination).is_err());
    assert!(destination.is_dir());
    assert_eq!(
        std::fs::read_to_string(destination.join("keep.txt")).unwrap(),
        "kept\n"
    );
    // The temporary file is cleaned up rather than left as debris.
    let leftovers: Vec<String> = std::fs::read_dir(root.path().join("exports"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["memory.json".to_string()]);
}

#[test]
fn a_failed_publication_leaves_the_destination_and_no_debris() {
    let (root, config) = open_root();
    // A directory occupies the destination. The export must refuse it and
    // leave no temporary file behind — the directory and its contents survive
    // untouched. (The `persist_noclobber` rename is the backstop for the
    // narrow race between this refusal and the publish; it can only ever fail
    // *before* replacing anything, which is exactly the guarantee asserted
    // here.)
    let destination = root.path().join("exports").join("memory.json");
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("keep.txt"), b"kept\n").unwrap();

    assert!(hiero::export::run(&config, &destination).is_err());
    assert!(destination.is_dir());
    assert_eq!(
        std::fs::read_to_string(destination.join("keep.txt")).unwrap(),
        "kept\n"
    );
    let leftovers: Vec<String> = std::fs::read_dir(root.path().join("exports"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["memory.json".to_string()]);
}

#[test]
fn export_writes_a_fresh_nested_destination() {
    let (root, config) = open_root();
    hieronymus::registry::Registry::open(&config)
        .unwrap()
        .create_series("book", "Book", "ja", "en", None)
        .unwrap();

    let destination = root
        .path()
        .join("out")
        .join("deep")
        .join("nested")
        .join("memory.json");
    let report = hiero::export::run(&config, &destination).unwrap();
    assert_eq!(report.output, destination);

    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&destination).unwrap()).unwrap();
    assert_eq!(document["format"], hiero::export::EXPORT_FORMAT);
    assert_eq!(document["tables"]["series"][0]["slug"], "book");
}

#[test]
fn export_reflects_one_committed_snapshot_under_a_concurrent_writer() {
    let (root, config) = open_root();
    let database = config.database_path();

    // The writer commits a `series` row and the `task_sessions` row that
    // references it in ONE transaction, over and over. A reader that ran each
    // table query in its own implicit transaction could observe the session
    // without its series; one read snapshot never can. The pair count is
    // bounded so the exports stay small — the point is the overlap, not volume.
    const PAIRS: u32 = 150;
    let done = Arc::new(AtomicBool::new(false));
    let writer_done = Arc::clone(&done);
    let writer = std::thread::spawn(move || {
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .busy_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        connection
            .execute_batch("pragma foreign_keys = on;")
            .unwrap();
        for index in 0..PAIRS {
            let slug = format!("series-{index}");
            connection.execute_batch("begin immediate").unwrap();
            connection
                .execute(
                    "insert into series (slug, title, default_source_language, \
                     default_target_language, created_at, updated_at) \
                     values (?1, ?1, 'ja', 'en', '2026-09-06T00:00:00Z', '2026-09-06T00:00:00Z')",
                    [&slug],
                )
                .unwrap();
            connection
                .execute(
                    "insert into task_sessions (series_slug, source_language, target_language, \
                     task_type, status, created_at, last_activity_at) \
                     values (?1, 'ja', 'en', 'translation', 'active', \
                     '2026-09-06T00:00:00Z', '2026-09-06T00:00:00Z')",
                    [&slug],
                )
                .unwrap();
            connection.execute_batch("commit").unwrap();
            // Spread the commits across the reader's exports instead of
            // finishing before the first one starts.
            std::thread::sleep(std::time::Duration::from_micros(300));
        }
        writer_done.store(true, Ordering::Release);
    });

    let mut exports = 0_u32;
    let mut round = 0_u32;
    // Export until the writer is finished (bounded, so a stalled writer fails
    // the test instead of hanging it).
    while round < 400 && !(done.load(Ordering::Acquire) && exports > 3) {
        let destination = root.path().join("snapshots").join(format!("{round}.json"));
        round += 1;
        if hiero::export::run(&config, &destination).is_err() {
            // A read-only reader can still lose a race on a busy root; that is
            // a refusal, not a torn export, so it proves nothing here.
            continue;
        }
        let document: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&destination).unwrap()).unwrap();
        // Keep the run cheap: each snapshot is checked and dropped.
        std::fs::remove_file(&destination).unwrap();
        exports += 1;
        let slugs: Vec<&str> = document["tables"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["slug"].as_str().unwrap())
            .collect();
        for session in document["tables"]["task_sessions"].as_array().unwrap() {
            let referenced = session["series_slug"].as_str().unwrap();
            assert!(
                slugs.contains(&referenced),
                "session references {referenced}, which the same export's series table \
                 does not contain: the two tables came from different snapshots"
            );
        }
    }

    writer.join().unwrap();
    assert!(
        exports > 3,
        "too few exports overlapped the writer: {exports}"
    );
    // A final export sees every committed pair.
    let final_destination = root.path().join("snapshots").join("final.json");
    hiero::export::run(&config, &final_destination).unwrap();
    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&final_destination).unwrap()).unwrap();
    assert_eq!(
        document["tables"]["task_sessions"]
            .as_array()
            .unwrap()
            .len(),
        PAIRS as usize
    );
}

/// A destination directly inside an existing directory still publishes, and
/// the report names exactly the path that now holds the document.
#[test]
fn export_report_names_the_destination_it_published() {
    let (root, config) = open_root();
    let destination = root.path().join("memory.json");
    let report = hiero::export::run(&config, &destination).unwrap();
    assert_eq!(report.output, destination);
    assert_eq!(report.format, hiero::export::EXPORT_FORMAT);
    assert_eq!(report.tables.len(), hiero::export::EXPORTED_TABLES.len());
    assert!(destination.is_file());
}

#[test]
#[cfg(unix)]
fn export_refuses_symlink_parent_traversal_before_normalizing() {
    let (root, config) = open_root();
    let outside = native_tempdir();
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let alias = outside.path().join("alias");
    std::os::unix::fs::symlink(&nested, &alias).unwrap();
    let destination = alias
        .join("..")
        .join(config.database_path().file_name().unwrap());
    let before = std::fs::read(config.database_path()).unwrap();
    assert!(matches!(
        hiero::export::run_overwriting(&config, &destination),
        Err(hiero::export::ExportError::UnsafeDestination(_))
    ));
    assert_eq!(std::fs::read(config.database_path()).unwrap(), before);
}

fn native_tempdir() -> tempfile::TempDir {
    tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
}
