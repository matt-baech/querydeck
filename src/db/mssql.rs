use std::time::Duration;

use futures_util::TryStreamExt;
use tiberius::{AuthMethod, Client, ColumnData, Config, FromSql, QueryItem};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use super::{MAX_ROWS, NULL, Profile, QueryOutcome, ResultSet, rows_message, split_go_batches};

type SqlClient = Client<Compat<TcpStream>>;

pub struct Conn {
    // tiberius needs `&mut` for every call, so the session sits behind a lock.
    // A running query holds it; cancel takes it once the query task is aborted.
    client: Mutex<Option<SqlClient>>,
}

impl Conn {
    pub async fn connect(profile: &Profile, password: &str) -> Result<Self, String> {
        let mut config = Config::new();
        config.host(&profile.host);
        config.port(profile.port);
        config.authentication(AuthMethod::sql_server(&profile.user, password));
        if !profile.database.trim().is_empty() {
            config.database(profile.database.trim());
        }
        if profile.trust_cert {
            config.trust_cert();
        }
        config.application_name("querydeck");

        let tcp = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(config.get_addr()))
            .await
            .map_err(|_| format!("Timed out connecting to {}", config.get_addr()))?
            .map_err(|e| format!("Could not reach {}: {e}", config.get_addr()))?;
        tcp.set_nodelay(true).map_err(|e| e.to_string())?;
        let client = Client::connect(config, tcp.compat_write()).await.map_err(|e| e.to_string())?;
        Ok(Self { client: Mutex::new(Some(client)) })
    }

    /// Rows of the first result set of `sql`.
    pub async fn query_rows(&self, sql: &str) -> Result<Vec<Vec<String>>, String> {
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or("Connection is closed.")?;
        let sets = run_batch(client, sql).await?;
        Ok(sets.into_iter().next().map(|s| s.rows).unwrap_or_default())
    }

    pub async fn run(&self, sql: &str) -> QueryOutcome {
        let mut guard = self.client.lock().await;
        let Some(client) = guard.as_mut() else {
            return QueryOutcome { messages: vec!["Connection is closed.".into()], failed: true, ..Default::default() };
        };

        let mut outcome = QueryOutcome::default();
        let batches = split_go_batches(sql);
        let multi = batches.len() > 1;
        for (i, batch) in batches.iter().enumerate() {
            let prefix = if multi { format!("Batch {}: ", i + 1) } else { String::new() };
            match run_batch(client, batch).await {
                Ok(sets) => {
                    if sets.is_empty() {
                        outcome.messages.push(format!("{prefix}Commands completed successfully."));
                    }
                    for set in &sets {
                        outcome.messages.push(rows_message(&prefix, set));
                    }
                    outcome.results.extend(sets);
                }
                Err(e) => {
                    outcome.messages.push(format!("{prefix}{e}"));
                    outcome.failed = true;
                    break;
                }
            }
        }
        outcome
    }

    pub async fn cancel(&self) -> Result<(), String> {
        let mut guard = self.client.lock().await;
        match guard.as_mut() {
            Some(client) => client.cancel_query().await.map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }

    pub async fn close(&self) {
        if let Some(client) = self.client.lock().await.take() {
            let _ = client.close().await;
        }
    }
}

/// Runs one T-SQL batch and collects every result set it produces.
async fn run_batch(c: &mut SqlClient, sql: &str) -> Result<Vec<ResultSet>, String> {
    let mut stream = c.simple_query(sql).await.map_err(|e| e.to_string())?;
    let mut sets: Vec<ResultSet> = Vec::new();
    while let Some(item) = stream.try_next().await.map_err(|e| e.to_string())? {
        match item {
            QueryItem::Metadata(meta) => sets.push(ResultSet {
                columns: meta.columns().iter().map(|c| c.name().to_string()).collect(),
                ..Default::default()
            }),
            QueryItem::Row(row) => {
                let Some(set) = sets.last_mut() else { continue };
                if set.rows.len() >= MAX_ROWS {
                    set.truncated = true;
                    continue;
                }
                set.rows.push(row.cells().map(|(_, d)| format_cell(d)).collect());
            }
        }
    }
    Ok(sets)
}

fn format_cell(d: &ColumnData<'static>) -> String {
    fn conv<'a, T: FromSql<'a> + ToString>(d: &'a ColumnData<'static>) -> String {
        match T::from_sql(d) {
            Ok(Some(v)) => v.to_string(),
            Ok(None) => NULL.into(),
            Err(e) => format!("<{e}>"),
        }
    }
    fn opt<T: ToString>(v: &Option<T>) -> String {
        v.as_ref().map(|x| x.to_string()).unwrap_or_else(|| NULL.into())
    }
    match d {
        ColumnData::U8(v) => opt(v),
        ColumnData::I16(v) => opt(v),
        ColumnData::I32(v) => opt(v),
        ColumnData::I64(v) => opt(v),
        ColumnData::F32(v) => opt(v),
        ColumnData::F64(v) => opt(v),
        ColumnData::Bit(v) => opt(&v.map(u8::from)),
        ColumnData::String(v) => opt(v),
        ColumnData::Guid(v) => opt(&v.map(|g| g.to_string().to_uppercase())),
        ColumnData::Binary(v) => match v {
            Some(b) => {
                let mut s = String::from("0x");
                for byte in b.iter() {
                    s.push_str(&format!("{byte:02X}"));
                }
                s
            }
            None => NULL.into(),
        },
        ColumnData::Numeric(_) => conv::<rust_decimal::Decimal>(d),
        ColumnData::Xml(v) => opt(&v.as_ref().map(|x| x.to_string())),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            conv::<chrono::NaiveDateTime>(d)
        }
        ColumnData::Date(_) => conv::<chrono::NaiveDate>(d),
        ColumnData::Time(_) => conv::<chrono::NaiveTime>(d),
        ColumnData::DateTimeOffset(_) => conv::<chrono::DateTime<chrono::FixedOffset>>(d),
    }
}
