//! Database access. Each driver lives in its own module and exposes the same
//! small surface: connect, run a script, cancel the running script.

pub mod catalog;
mod mssql;
mod postgres;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Rows beyond this are dropped from a result set to keep memory bounded.
pub const MAX_ROWS: usize = 100_000;

pub const NULL: &str = "NULL";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Driver {
    SqlServer,
    Postgres,
}

impl Driver {
    pub const ALL: [Driver; 2] = [Driver::SqlServer, Driver::Postgres];

    pub fn default_port(self) -> u16 {
        match self {
            Driver::SqlServer => 1433,
            Driver::Postgres => 5432,
        }
    }

    pub fn default_user(self) -> &'static str {
        match self {
            Driver::SqlServer => "sa",
            Driver::Postgres => "postgres",
        }
    }

    pub fn default_database(self) -> &'static str {
        match self {
            Driver::SqlServer => "master",
            Driver::Postgres => "postgres",
        }
    }
}

impl fmt::Display for Driver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Driver::SqlServer => "SQL Server",
            Driver::Postgres => "PostgreSQL",
        })
    }
}

/// Everything needed to open a connection, except the password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub driver: Driver,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    /// SQL Server only: accept the server's self-signed certificate.
    #[serde(default)]
    pub trust_cert: bool,
}

impl Profile {
    pub fn new(driver: Driver) -> Self {
        Self {
            name: String::new(),
            driver,
            host: "localhost".into(),
            port: driver.default_port(),
            user: driver.default_user().into(),
            database: driver.default_database().into(),
            trust_cert: true,
        }
    }

    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() {
            format!("{}@{}:{}", self.user, self.host, self.port)
        } else {
            self.name.clone()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default)]
pub struct QueryOutcome {
    pub results: Vec<ResultSet>,
    pub messages: Vec<String>,
    pub elapsed: Duration,
    /// True if any statement failed; the messages say which.
    pub failed: bool,
    /// The session's database after the script ran (it may have run `USE`).
    pub current_database: Option<String>,
    /// Rows changed by INSERT, UPDATE, DELETE and the like, when the driver
    /// reports it (PostgreSQL does; tiberius drops SQL Server's counts).
    pub rows_affected: Option<u64>,
}

/// An open connection. Cheap to clone; clones share the same session.
#[derive(Clone)]
pub enum Connection {
    SqlServer(Arc<mssql::Conn>),
    Postgres(Arc<postgres::Conn>),
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Connection::SqlServer(_) => f.write_str("Connection(SqlServer)"),
            Connection::Postgres(_) => f.write_str("Connection(Postgres)"),
        }
    }
}

/// Result of a successful connect: the connection plus the server's version banner.
#[derive(Debug, Clone)]
pub struct Connected {
    pub conn: Connection,
    pub server_version: String,
    pub database: String,
}

pub async fn connect(profile: Profile, password: String) -> Result<Connected, String> {
    match profile.driver {
        Driver::SqlServer => {
            let conn = mssql::Conn::connect(&profile, &password).await?;
            let database = conn.current_database().await.unwrap_or_default();
            let server_version = conn.server_version().await.unwrap_or_default();
            Ok(Connected { conn: Connection::SqlServer(Arc::new(conn)), server_version, database })
        }
        Driver::Postgres => {
            let conn = postgres::Conn::connect(&profile, &password).await?;
            let database = conn.current_database().await.unwrap_or_default();
            let server_version = conn.server_version().await.unwrap_or_default();
            Ok(Connected { conn: Connection::Postgres(Arc::new(conn)), server_version, database })
        }
    }
}

impl Connection {
    pub async fn run(self, sql: String) -> QueryOutcome {
        let start = std::time::Instant::now();
        let mut outcome = match &self {
            Connection::SqlServer(c) => c.run(&sql).await,
            Connection::Postgres(c) => c.run(&sql).await,
        };
        outcome.elapsed = start.elapsed();
        outcome.current_database = match &self {
            Connection::SqlServer(c) => c.current_database().await,
            Connection::Postgres(c) => c.current_database().await,
        };
        outcome
    }

    /// Asks the server to stop the running query. Call after aborting the task
    /// that was running it.
    pub async fn cancel(self) -> Result<(), String> {
        match &self {
            Connection::SqlServer(c) => c.cancel().await,
            Connection::Postgres(c) => c.cancel().await,
        }
    }

    pub fn driver(&self) -> Driver {
        match self {
            Connection::SqlServer(_) => Driver::SqlServer,
            Connection::Postgres(_) => Driver::Postgres,
        }
    }

    pub async fn query_rows(&self, sql: &str) -> Result<Vec<Vec<String>>, String> {
        match self {
            Connection::SqlServer(c) => c.query_rows(sql).await,
            Connection::Postgres(c) => c.query_rows(sql).await,
        }
    }

    pub async fn close(self) {
        if let Connection::SqlServer(c) = &self {
            c.close().await;
        }
        // Postgres closes when the last clone of the client is dropped.
    }
}

/// Splits a script on lines consisting solely of `GO`, like SSMS and Azure Data Studio.
pub fn split_go_batches(sql: &str) -> Vec<String> {
    let mut batches = Vec::new();
    let mut cur = String::new();
    for line in sql.lines() {
        if line.trim().eq_ignore_ascii_case("go") {
            if !cur.trim().is_empty() {
                batches.push(std::mem::take(&mut cur));
            }
            cur.clear();
        } else {
            cur.push_str(line);
            cur.push('\n');
        }
    }
    if !cur.trim().is_empty() {
        batches.push(cur);
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_go_lines_only() {
        let b = split_go_batches("select 1\nGO\n  go  \nselect 'go'\ngo");
        assert_eq!(b, vec!["select 1\n", "select 'go'\n"]);
    }

    #[test]
    fn no_go_is_one_batch() {
        assert_eq!(split_go_batches("select 1;\nselect 2;"), vec!["select 1;\nselect 2;\n"]);
    }
}
