use std::{
    fs::{self, OpenOptions},
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddrV4, TcpListener},
    path::Path,
};

use serde::{Deserialize, Serialize};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};

use crate::{agent::agent_plugins, config::HieronymusConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorCheck {
    fn new(name: impl Into<String>, status: CheckStatus, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
        }
    }
}

/// Observes the current installation without changing it.
///
/// Checks are independent snapshots. A successful result does not guarantee that a later startup
/// can still acquire the same filesystem or network resources.
pub async fn run_doctor(config: &HieronymusConfig) -> DoctorReport {
    let config = config.clone();
    run_doctor_batch(move || run_doctor_sync(&config, &SystemProbes)).await
}

async fn run_doctor_batch(batch: impl FnOnce() -> DoctorReport + Send + 'static) -> DoctorReport {
    match tokio::task::spawn_blocking(batch).await {
        Ok(report) => report,
        Err(_) => DoctorReport {
            checks: vec![DoctorCheck::new(
                "doctor-runtime",
                CheckStatus::Fail,
                "diagnostic observation worker did not complete",
            )],
        },
    }
}

fn run_doctor_sync(config: &HieronymusConfig, probes: &impl DoctorProbes) -> DoctorReport {
    let mut checks = Vec::new();

    for (name, path) in [
        ("provider.conf", config.provider_config_path()),
        ("dream.conf", config.dream_config_path()),
        ("ingest.conf", config.ingest_config_path()),
        ("semantic.conf", config.semantic_config_path()),
    ] {
        checks.push(check_toml_config(name, &path));
    }

    let database = observe_database(&config.database_path(), probes);
    checks.push(database_check(&database));
    checks.extend(check_agent_plugins());
    checks.push(check_bind_port(config, probes));
    checks.push(check_derived_index(config, &database, probes));

    DoctorReport { checks }
}

fn check_toml_config(filename: &str, path: &Path) -> DoctorCheck {
    let name = format!("config:{filename}");
    match fs::read_to_string(path) {
        Ok(contents) => match contents.parse::<toml::Table>() {
            Ok(_) => DoctorCheck::new(name, CheckStatus::Ok, format!("{filename} is valid TOML")),
            Err(_) => DoctorCheck::new(
                name,
                CheckStatus::Fail,
                format!("{filename} is not valid TOML"),
            ),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => DoctorCheck::new(
            name,
            CheckStatus::Warn,
            format!("{filename} is absent; built-in defaults apply where supported"),
        ),
        Err(_) => DoctorCheck::new(
            name,
            CheckStatus::Fail,
            format!("{filename} cannot currently be read"),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DatabaseObservation {
    path_kind: DatabasePathKind,
    readable_sqlite: bool,
    writable_file: bool,
    parent: DirectoryObservation,
}

impl DatabaseObservation {
    fn healthy(&self) -> bool {
        self.path_kind == DatabasePathKind::RegularFile
            && self.readable_sqlite
            && self.writable_file
            && self.parent == DirectoryObservation::Writable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabasePathKind {
    Missing,
    RegularFile,
    Obstructed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectoryPathKind {
    Missing,
    Directory,
    Obstructed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectoryObservation {
    Writable,
    Unwritable,
    Obstructed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindObservation {
    Available,
    InUse,
    Unavailable,
}

trait DoctorProbes: Sync {
    fn sqlite_readable(&self, path: &Path) -> bool;
    fn file_writable(&self, path: &Path) -> bool;
    fn directory_observation(&self, path: &Path) -> DirectoryObservation;
    fn bind_port(&self, port: u16) -> BindObservation;
}

struct SystemProbes;

impl DoctorProbes for SystemProbes {
    fn sqlite_readable(&self, path: &Path) -> bool {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return false;
        };
        runtime.block_on(async {
            let options = SqliteConnectOptions::new()
                .filename(path)
                .read_only(true)
                .create_if_missing(false);
            let Ok(mut connection) = SqliteConnection::connect_with(&options).await else {
                return false;
            };
            sqlx::query("SELECT name FROM sqlite_schema LIMIT 1")
                .fetch_optional(&mut connection)
                .await
                .is_ok()
        })
    }

    fn file_writable(&self, path: &Path) -> bool {
        OpenOptions::new().read(true).write(true).open(path).is_ok()
    }

    fn directory_observation(&self, path: &Path) -> DirectoryObservation {
        directory_write_observation(path)
    }

    fn bind_port(&self, port: u16) -> BindObservation {
        match TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)) {
            Ok(listener) => {
                drop(listener);
                BindObservation::Available
            }
            Err(error) if error.kind() == ErrorKind::AddrInUse => BindObservation::InUse,
            Err(_) => BindObservation::Unavailable,
        }
    }
}

fn observe_database(path: &Path, probes: &impl DoctorProbes) -> DatabaseObservation {
    let mut path_kind = classify_database_path(path);
    let effective_path = if path_kind == DatabasePathKind::RegularFile {
        match fs::canonicalize(path) {
            Ok(path) => Some(path),
            Err(_) => {
                path_kind = DatabasePathKind::Obstructed;
                None
            }
        }
    } else {
        None
    };
    let regular_file = path_kind == DatabasePathKind::RegularFile;
    let probe_path = effective_path.as_deref().unwrap_or(path);
    DatabaseObservation {
        path_kind,
        readable_sqlite: regular_file && probes.sqlite_readable(probe_path),
        writable_file: regular_file && probes.file_writable(probe_path),
        parent: probe_path
            .parent()
            .map_or(DirectoryObservation::Obstructed, |parent| {
                probes.directory_observation(parent)
            }),
    }
}

fn database_check(observation: &DatabaseObservation) -> DoctorCheck {
    if observation.path_kind == DatabasePathKind::Obstructed
        || observation.parent == DirectoryObservation::Obstructed
    {
        return DoctorCheck::new(
            "database",
            CheckStatus::Fail,
            "database path is obstructed by an invalid symlink or non-directory component",
        );
    }
    if observation.path_kind == DatabasePathKind::Missing {
        return if observation.parent == DirectoryObservation::Writable {
            DoctorCheck::new(
                "database",
                CheckStatus::Warn,
                "database is absent; its nearest existing ancestor is currently writable",
            )
        } else {
            DoctorCheck::new(
                "database",
                CheckStatus::Fail,
                "database is absent and no writable existing ancestor was observed",
            )
        };
    }
    if !observation.readable_sqlite {
        return DoctorCheck::new(
            "database",
            CheckStatus::Fail,
            "database is not currently readable as SQLite",
        );
    }
    if !observation.writable_file {
        return DoctorCheck::new(
            "database",
            CheckStatus::Fail,
            "database is readable but its file is not currently writable",
        );
    }
    if observation.parent != DirectoryObservation::Writable {
        return DoctorCheck::new(
            "database",
            CheckStatus::Fail,
            "database is readable but its parent directory is not currently writable",
        );
    }
    DoctorCheck::new(
        "database",
        CheckStatus::Ok,
        "database is readable and both its file and parent directory are currently writable",
    )
}

fn classify_database_path(path: &Path) -> DatabasePathKind {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => match fs::metadata(path) {
            Ok(target) if target.is_file() => DatabasePathKind::RegularFile,
            _ => DatabasePathKind::Obstructed,
        },
        Ok(metadata) if metadata.is_file() => DatabasePathKind::RegularFile,
        Ok(_) => DatabasePathKind::Obstructed,
        Err(error) if error.kind() == ErrorKind::NotFound => DatabasePathKind::Missing,
        Err(_) => DatabasePathKind::Obstructed,
    }
}

fn classify_directory_path(path: &Path) -> DirectoryPathKind {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => match fs::metadata(path) {
            Ok(target) if target.is_dir() => DirectoryPathKind::Directory,
            _ => DirectoryPathKind::Obstructed,
        },
        Ok(metadata) if metadata.is_dir() => DirectoryPathKind::Directory,
        Ok(_) => DirectoryPathKind::Obstructed,
        Err(error) if error.kind() == ErrorKind::NotFound => DirectoryPathKind::Missing,
        Err(_) => DirectoryPathKind::Obstructed,
    }
}

fn directory_write_observation(path: &Path) -> DirectoryObservation {
    let mut candidate = path;
    loop {
        match classify_directory_path(candidate) {
            DirectoryPathKind::Directory => break,
            DirectoryPathKind::Missing => {
                let Some(parent) = candidate.parent() else {
                    return DirectoryObservation::Obstructed;
                };
                candidate = parent;
            }
            DirectoryPathKind::Obstructed => return DirectoryObservation::Obstructed,
        }
    }
    match tempfile::tempfile_in(candidate) {
        Ok(file) => {
            drop(file);
            DirectoryObservation::Writable
        }
        Err(_) => DirectoryObservation::Unwritable,
    }
}

fn check_agent_plugins() -> Vec<DoctorCheck> {
    agent_plugins()
        .into_iter()
        .map(|plugin| {
            let availability = plugin.detect();
            let available = availability.detect_paths.iter().any(|path| path.exists());
            let (status, detail) = if availability.installed {
                (CheckStatus::Ok, "Hieronymus integration is installed")
            } else if available {
                (
                    CheckStatus::Warn,
                    "agent is available but Hieronymus integration is not installed",
                )
            } else {
                (CheckStatus::Ok, "agent is not detected on this system")
            };
            DoctorCheck::new(format!("agent:{}", plugin.name()), status, detail)
        })
        .collect()
}

fn check_bind_port(config: &HieronymusConfig, probes: &impl DoctorProbes) -> DoctorCheck {
    let port = config.resolve_port(None);
    match probes.bind_port(port) {
        BindObservation::Available => DoctorCheck::new(
            "bind-port",
            CheckStatus::Ok,
            format!("127.0.0.1:{port} is currently bindable"),
        ),
        BindObservation::InUse => DoctorCheck::new(
            "bind-port",
            CheckStatus::Warn,
            format!("127.0.0.1:{port} is currently in use"),
        ),
        BindObservation::Unavailable => DoctorCheck::new(
            "bind-port",
            CheckStatus::Fail,
            format!("127.0.0.1:{port} cannot currently be bound"),
        ),
    }
}

fn check_derived_index(
    config: &HieronymusConfig,
    database: &DatabaseObservation,
    probes: &impl DoctorProbes,
) -> DoctorCheck {
    let index_path = config.lancedb_dir();
    let index_kind = classify_directory_path(&index_path);

    if index_kind == DirectoryPathKind::Obstructed {
        return DoctorCheck::new(
            "derived-index",
            CheckStatus::Fail,
            "derived index path is obstructed by an invalid symlink or non-directory component",
        );
    }

    match probes.directory_observation(&index_path) {
        DirectoryObservation::Writable => {}
        DirectoryObservation::Unwritable => {
            return DoctorCheck::new(
                "derived-index",
                CheckStatus::Fail,
                "derived index destination is not currently writable",
            );
        }
        DirectoryObservation::Obstructed => {
            return DoctorCheck::new(
                "derived-index",
                CheckStatus::Fail,
                "derived index path is obstructed by an invalid symlink or non-directory component",
            );
        }
    }

    if database.path_kind != DatabasePathKind::Missing && !database.healthy() {
        return DoctorCheck::new(
            "derived-index",
            CheckStatus::Fail,
            "derived index is unavailable because the authoritative database health check failed",
        );
    }

    match (
        index_kind == DirectoryPathKind::Directory,
        database.healthy(),
    ) {
        (true, true) => DoctorCheck::new(
            "derived-index",
            CheckStatus::Ok,
            "derived index exists and can be rebuilt from the authoritative database",
        ),
        (true, false) => DoctorCheck::new(
            "derived-index",
            CheckStatus::Fail,
            "derived index exists without an authoritative database to rebuild it",
        ),
        (false, true) => DoctorCheck::new(
            "derived-index",
            CheckStatus::Warn,
            "derived index is absent and can be rebuilt from the authoritative database",
        ),
        (false, false) => DoctorCheck::new(
            "derived-index",
            CheckStatus::Ok,
            "no derived index exists before database initialization",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::mpsc, time::Duration};

    use crate::config::HieronymusConfig;

    use super::{
        CheckStatus, DirectoryObservation, DoctorProbes, DoctorReport, SystemProbes,
        check_derived_index, check_toml_config, database_check, observe_database, run_doctor_batch,
    };

    struct FixedProbes {
        sqlite_readable: bool,
        file_writable: bool,
        directory_writable: bool,
    }

    #[cfg(unix)]
    struct PathCheckingProbes {
        file: std::path::PathBuf,
        parent: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl DoctorProbes for PathCheckingProbes {
        fn sqlite_readable(&self, path: &std::path::Path) -> bool {
            assert_eq!(path, self.file);
            true
        }

        fn file_writable(&self, path: &std::path::Path) -> bool {
            assert_eq!(path, self.file);
            true
        }

        fn directory_observation(&self, path: &std::path::Path) -> DirectoryObservation {
            assert_eq!(path, self.parent);
            DirectoryObservation::Writable
        }

        fn bind_port(&self, _port: u16) -> super::BindObservation {
            super::BindObservation::Available
        }
    }

    impl DoctorProbes for FixedProbes {
        fn sqlite_readable(&self, _path: &std::path::Path) -> bool {
            self.sqlite_readable
        }

        fn file_writable(&self, _path: &std::path::Path) -> bool {
            self.file_writable
        }

        fn directory_observation(&self, _path: &std::path::Path) -> DirectoryObservation {
            if self.directory_writable {
                DirectoryObservation::Writable
            } else {
                DirectoryObservation::Unwritable
            }
        }

        fn bind_port(&self, _port: u16) -> super::BindObservation {
            super::BindObservation::Available
        }
    }

    #[test]
    fn config_check_does_not_echo_invalid_contents() {
        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let path = temporary.path().join("provider.conf");
        let secret = "super-secret-api-key";
        fs::write(&path, format!("api_key = '{secret}'\n[broken"))
            .expect("invalid config should be written");

        let check = check_toml_config("provider.conf", &path);

        assert_eq!(check.status, CheckStatus::Fail);
        assert!(!check.detail.contains(secret));
    }

    #[cfg(unix)]
    #[test]
    fn database_decision_rejects_injected_effective_write_denial() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let database = temporary.path().join("hieronymus.db");
        fs::write(&database, "fixture").expect("database fixture should be written");
        fs::set_permissions(&database, fs::Permissions::from_mode(0o444))
            .expect("fixture permissions should be set");
        let probes = FixedProbes {
            sqlite_readable: true,
            file_writable: false,
            directory_writable: true,
        };

        let observation = observe_database(&database, &probes);
        let check = database_check(&observation);

        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.detail.contains("not currently writable"));
    }

    #[cfg(unix)]
    #[test]
    fn database_symlink_probes_the_resolved_file_and_journal_parent() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let target_parent = temporary.path().join("target");
        let link_parent = temporary.path().join("links");
        fs::create_dir(&target_parent).expect("target parent should be created");
        fs::create_dir(&link_parent).expect("link parent should be created");
        let target = target_parent.join("authoritative.db");
        fs::write(&target, "fixture").expect("target should be written");
        let link = link_parent.join("hieronymus.db");
        symlink(&target, &link).expect("database symlink should be created");
        let probes = PathCheckingProbes {
            file: fs::canonicalize(&target).expect("target should canonicalize"),
            parent: fs::canonicalize(&target_parent).expect("parent should canonicalize"),
        };

        let observation = observe_database(&link, &probes);

        assert!(observation.healthy());
    }

    #[test]
    fn derived_index_requires_a_writable_destination() {
        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let data_root = temporary.path().join("data");
        fs::create_dir(&data_root).expect("data root should be created");
        fs::write(data_root.join("hieronymus.db"), "fixture")
            .expect("database fixture should be written");
        let config = HieronymusConfig::load(Some(data_root)).expect("configuration should resolve");
        let probes = FixedProbes {
            sqlite_readable: true,
            file_writable: true,
            directory_writable: false,
        };
        let database = observe_database(&config.database_path(), &probes);

        let check = check_derived_index(&config, &database, &probes);

        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.detail.contains("not currently writable"));
    }

    #[test]
    fn derived_index_does_not_claim_rebuildability_when_database_health_fails() {
        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let data_root = temporary.path().join("data");
        fs::create_dir(&data_root).expect("data root should be created");
        fs::write(data_root.join("hieronymus.db"), "fixture")
            .expect("database fixture should be written");
        let config = HieronymusConfig::load(Some(data_root)).expect("configuration should resolve");
        let probes = FixedProbes {
            sqlite_readable: true,
            file_writable: false,
            directory_writable: true,
        };
        let database = observe_database(&config.database_path(), &probes);

        assert_eq!(database_check(&database).status, CheckStatus::Fail);
        let derived = check_derived_index(&config, &database, &probes);
        assert_eq!(derived.status, CheckStatus::Fail);
        assert!(!derived.detail.contains("rebuild"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_observation_batch_does_not_stall_the_tokio_worker() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let doctor = tokio::spawn(run_doctor_batch(move || {
            let _ = started_tx.send(());
            release_rx
                .recv()
                .expect("test should release blocked probe");
            DoctorReport { checks: Vec::new() }
        }));

        started_rx.await.expect("blocking batch should start");
        let unrelated_task = tokio::spawn(async {
            tokio::task::yield_now().await;
            42
        });
        let unrelated = tokio::time::timeout(Duration::from_secs(1), unrelated_task)
            .await
            .expect("Tokio worker should remain responsive")
            .expect("unrelated task should join");
        assert_eq!(unrelated, 42);
        release_tx
            .send(())
            .expect("blocking batch should be released");
        doctor.await.expect("doctor task should join");
    }

    #[tokio::test]
    async fn failed_observation_worker_becomes_a_safe_report() {
        let report = run_doctor_batch(|| panic!("test-only worker panic")).await;

        assert_eq!(report.checks.len(), 1);
        assert_eq!(report.checks[0].name, "doctor-runtime");
        assert_eq!(report.checks[0].status, CheckStatus::Fail);
        assert_eq!(
            report.checks[0].detail,
            "diagnostic observation worker did not complete"
        );
    }

    #[test]
    fn system_write_probes_preserve_contents_and_remove_temporary_artifacts() {
        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let database = temporary.path().join("hieronymus.db");
        fs::write(&database, "unchanged").expect("database fixture should be written");
        let missing_destination = temporary.path().join("derived").join("nested");

        assert!(SystemProbes.file_writable(&database));
        assert_eq!(
            SystemProbes.directory_observation(&missing_destination),
            DirectoryObservation::Writable
        );

        assert_eq!(
            fs::read_to_string(&database).expect("database fixture should remain readable"),
            "unchanged"
        );
        assert!(!missing_destination.exists());
        let entries = fs::read_dir(temporary.path())
            .expect("temporary root should remain readable")
            .collect::<Result<Vec<_>, _>>()
            .expect("temporary root entries should be readable");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path(), database);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn system_file_probe_observes_real_effective_write_denial() {
        assert!(!SystemProbes.file_writable(std::path::Path::new("/proc/self/status")));
    }
}
