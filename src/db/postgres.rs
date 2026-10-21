use std::time::Duration;

use futures_util::{StreamExt, pin_mut};
use tokio_postgres::{CancelToken, Client, NoTls, SimpleQueryMessage};

use super::{MAX_ROWS, NULL, Profile, QueryOutcome, ResultSet};

pub struct Conn {
    client: Client,
    cancel: CancelToken,
}

impl Conn {
    pub async fn connect(profile: &Profile, password: &str) -> Result<Self, String> {
        let mut config = tokio_postgres::Config::new();
        config
            .host(&profile.host)
            .port(profile.port)
            .user(&profile.user)
            .password(password)
            .application_name("litedb")
            .connect_timeout(Duration::from_secs(10));
        if !profile.database.trim().is_empty() {
            config.dbname(profile.database.trim());
        }

        // TLS is not supported yet; fine for local servers and Docker containers.
        let (client, connection) = config.connect(NoTls).await.map_err(|e| describe(&e))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let cancel = client.cancel_token();
        Ok(Self { client, cancel })
    }

    pub async fn server_version(&self) -> Option<String> {
        self.scalar("SELECT version()").await
    }

    pub async fn current_database(&self) -> Option<String> {
        self.scalar("SELECT current_database()").await
    }

    /// Rows of the first result set of `sql`.
    pub async fn query_rows(&self, sql: &str) -> Result<Vec<Vec<String>>, String> {
        let msgs = self.client.simple_query(sql).await.map_err(|e| describe(&e))?;
        Ok(msgs
            .into_iter()
            .filter_map(|m| match m {
                SimpleQueryMessage::Row(r) => {
                    Some((0..r.len()).map(|i| r.get(i).unwrap_or(NULL).to_string()).collect())
                }
                _ => None,
            })
            .collect())
    }

    /// First column of the first row, or `None` on error or no rows.
    async fn scalar(&self, sql: &str) -> Option<String> {
        let msgs = self.client.simple_query(sql).await.ok()?;
        msgs.into_iter().find_map(|m| match m {
            SimpleQueryMessage::Row(r) => r.get(0).map(str::to_string),
            _ => None,
        })
    }

    pub async fn run(&self, sql: &str) -> QueryOutcome {
        let mut outcome = QueryOutcome::default();
        if let Err(e) = self.run_inner(sql, &mut outcome).await {
            outcome.messages.push(describe(&e));
            outcome.failed = true;
        }
        if outcome.messages.is_empty() && !outcome.failed {
            outcome.messages.push("Commands completed successfully.".into());
        }
        outcome
    }

    async fn run_inner(&self, sql: &str, outcome: &mut QueryOutcome) -> Result<(), tokio_postgres::Error> {
        let stream = self.client.simple_query_raw(sql).await?;
        pin_mut!(stream);
        // Rows arrive after a RowDescription; CommandComplete closes the statement.
        let mut open: Option<ResultSet> = None;
        while let Some(msg) = stream.next().await {
            match msg? {
                SimpleQueryMessage::RowDescription(cols) => {
                    open = Some(ResultSet {
                        columns: cols.iter().map(|c| c.name().to_string()).collect(),
                        ..Default::default()
                    });
                }
                SimpleQueryMessage::Row(row) => {
                    let set = open.get_or_insert_with(|| ResultSet {
                        columns: row.columns().iter().map(|c| c.name().to_string()).collect(),
                        ..Default::default()
                    });
                    if set.rows.len() >= MAX_ROWS {
                        set.truncated = true;
                        continue;
                    }
                    set.rows.push((0..row.len()).map(|i| row.get(i).unwrap_or(NULL).to_string()).collect());
                }
                SimpleQueryMessage::CommandComplete(n) => match open.take() {
                    Some(set) => {
                        let truncated =
                            if set.truncated { format!(" (only the first {MAX_ROWS} kept)") } else { String::new() };
                        outcome.messages.push(format!("({} row(s) returned){truncated}", set.rows.len()));
                        outcome.results.push(set);
                    }
                    None => outcome.messages.push(format!("({n} row(s) affected)")),
                },
                _ => {}
            }
        }
        if let Some(set) = open.take() {
            outcome.results.push(set);
        }
        Ok(())
    }

    pub async fn cancel(&self) -> Result<(), String> {
        self.cancel.cancel_query(NoTls).await.map_err(|e| describe(&e))
    }
}

/// Postgres errors print as "db error" unless you dig out the server's message.
fn describe(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => {
            let mut s = format!("{}: {}", db.severity(), db.message());
            if let Some(detail) = db.detail() {
                s.push_str(&format!("\nDETAIL: {detail}"));
            }
            if let Some(hint) = db.hint() {
                s.push_str(&format!("\nHINT: {hint}"));
            }
            s
        }
        None => e.to_string(),
    }
}
