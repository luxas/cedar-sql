//! Provisioning a Postgres for the test suites and the differential tests
//! (feature `testing`).
//!
//! [`SharedPostgres::get`] returns one server per process: the one at
//! `CEDAR_SQL_PG_URL` when that variable is set (CI runs a `postgres` service),
//! otherwise an embedded server started on first use and stopped when the
//! process exits. Every test or fuzz input should run inside
//! `BEGIN … ROLLBACK` (see [`crate::backend::Backend::begin`]), so the database
//! stays empty between inputs.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use postgresql_embedded::blocking::PostgreSQL;
use postgresql_embedded::{Settings, VersionReq};

use crate::backend::postgres::PgBackend;
use crate::{Error, Result};

/// The environment variable naming an existing Postgres to use.
pub const PG_URL_ENV: &str = "CEDAR_SQL_PG_URL";

/// The environment variable overriding where the embedded server is installed
/// (default: `~/.theseus/postgresql`); its data directory then lives under the
/// same directory instead of the system temporary directory.
pub const PG_INSTALL_DIR_ENV: &str = "CEDAR_SQL_PG_INSTALL_DIR";

/// The Postgres major version the embedded server installs.
const EMBEDDED_VERSION: &str = "=18.*";

/// The name of the database the embedded server creates.
const DATABASE: &str = "cedar_sql";

/// The Postgres shared by every test in this process.
pub struct SharedPostgres {
    url: String,
}

static SHARED: OnceLock<std::result::Result<SharedPostgres, String>> = OnceLock::new();
static EMBEDDED: Mutex<Option<PostgreSQL>> = Mutex::new(None);

impl SharedPostgres {
    /// The process-wide Postgres, provisioned on first use.
    ///
    /// # Errors
    ///
    /// When neither `CEDAR_SQL_PG_URL` is set nor an embedded server can be
    /// installed and started; the error is the same for every later call.
    pub fn get() -> Result<&'static SharedPostgres> {
        SHARED
            .get_or_init(|| Self::provision().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| Error::Provision(e.clone()))
    }

    /// The connection URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Opens a new connection.
    pub fn connect(&self) -> Result<PgBackend> {
        PgBackend::connect(&self.url)
    }

    fn provision() -> Result<Self> {
        if let Ok(url) = std::env::var(PG_URL_ENV) {
            return Ok(Self { url });
        }
        let mut settings = Settings {
            version: VersionReq::parse(EMBEDDED_VERSION)
                .map_err(|e| Error::Provision(e.to_string()))?,
            temporary: true,
            ..Settings::default()
        };
        if let Some(dir) = std::env::var_os(PG_INSTALL_DIR_ENV) {
            let dir = PathBuf::from(dir);
            settings.data_dir = dir.join("data").join(format!(
                "{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos())
            ));
            settings.installation_dir = dir;
        }
        let mut server = PostgreSQL::new(settings);
        server
            .setup()
            .map_err(|e| Error::Provision(format!("setup: {e}")))?;
        server
            .start()
            .map_err(|e| Error::Provision(format!("start: {e}")))?;
        server
            .create_database(DATABASE)
            .map_err(|e| Error::Provision(format!("create database: {e}")))?;
        let url = server.settings().url(DATABASE);
        *EMBEDDED.lock().unwrap_or_else(|p| p.into_inner()) = Some(server);
        // The server is a separate process: stop it when this one exits, since
        // a `static` is never dropped.
        // SAFETY: `atexit` with an `extern "C"` function of the right signature.
        #[allow(unsafe_code)]
        unsafe {
            libc::atexit(stop_embedded);
        }
        Ok(Self { url })
    }
}

extern "C" fn stop_embedded() {
    if let Ok(mut guard) = EMBEDDED.try_lock()
        && let Some(server) = guard.take()
    {
        // Errors cannot be reported meaningfully at exit.
        let _ = server.stop();
    }
}
