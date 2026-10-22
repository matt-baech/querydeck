//! Metadata queries for the Object Explorer: databases, objects, columns and
//! object definitions, for each supported driver.

use std::fmt;

use super::{Connection, Driver};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Table,
    View,
    Procedure,
    Function,
    Trigger,
}

impl ObjectKind {
    pub const ALL: [ObjectKind; 5] =
        [ObjectKind::Table, ObjectKind::View, ObjectKind::Procedure, ObjectKind::Function, ObjectKind::Trigger];

    pub fn folder_name(self) -> &'static str {
        match self {
            ObjectKind::Table => "Tables",
            ObjectKind::View => "Views",
            ObjectKind::Procedure => "Stored Procedures",
            ObjectKind::Function => "Functions",
            ObjectKind::Trigger => "Triggers",
        }
    }

    /// Whether the object has rows that "Select Top 1000" can show.
    pub fn is_selectable(self) -> bool {
        matches!(self, ObjectKind::Table | ObjectKind::View)
    }

    /// Whether the object has source code that can be scripted.
    pub fn has_definition(self) -> bool {
        !matches!(self, ObjectKind::Table)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbObject {
    pub kind: ObjectKind,
    pub schema: String,
    pub name: String,
    /// Triggers: the table they belong to.
    pub parent: Option<String>,
    /// Server-side id (object_id / oid), used for follow-up lookups.
    pub id: String,
}

impl DbObject {
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
}

impl fmt::Display for ColumnInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}{}, {})",
            self.name,
            if self.primary_key { "PK, " } else { "" },
            self.data_type,
            if self.nullable { "null" } else { "not null" }
        )
    }
}

/// The databases the user can access, with whether each is a built-in
/// system database (master, model, msdb, tempdb).
pub async fn databases(conn: Connection) -> Result<Vec<(String, bool)>, String> {
    let sql = match conn.driver() {
        Driver::SqlServer => {
            "SELECT name, CASE WHEN database_id <= 4 THEN 1 ELSE 0 END FROM sys.databases \
             WHERE HAS_DBACCESS(name) = 1 ORDER BY name"
        }
        // A Postgres connection can only see objects in its own database.
        Driver::Postgres => "SELECT current_database(), 0",
    };
    Ok(conn
        .query_rows(sql)
        .await?
        .into_iter()
        .filter_map(|r| {
            let [name, system]: [String; 2] = r.try_into().ok()?;
            Some((name, is_true(&system)))
        })
        .collect())
}

pub async fn objects(conn: Connection, database: String) -> Result<Vec<DbObject>, String> {
    let sql = match conn.driver() {
        Driver::SqlServer => format!(
            "SELECT s.name, o.name, RTRIM(o.type), OBJECT_NAME(o.parent_object_id, DB_ID({db_str})), o.object_id \
             FROM {db}.sys.objects o JOIN {db}.sys.schemas s ON s.schema_id = o.schema_id \
             WHERE o.is_ms_shipped = 0 AND o.type IN ('U','V','P','FN','IF','TF','TR') \
             ORDER BY s.name, o.name",
            db = mssql_ident(&database),
            db_str = mssql_str(&database),
        ),
        Driver::Postgres => "\
            SELECT n.nspname, c.relname, CASE WHEN c.relkind IN ('v','m') THEN 'V' ELSE 'U' END, NULL, c.oid::text \
            FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
            WHERE c.relkind IN ('r','p','v','m') AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' \
            UNION ALL \
            SELECT n.nspname, p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')', \
                   CASE p.prokind WHEN 'p' THEN 'P' ELSE 'FN' END, NULL, p.oid::text \
            FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
            WHERE p.prokind IN ('f','p') AND n.nspname NOT IN ('pg_catalog','information_schema') \
              AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = p.oid AND d.deptype = 'e') \
            UNION ALL \
            SELECT n.nspname, t.tgname, 'TR', c.relname, t.oid::text \
            FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
            WHERE NOT t.tgisinternal \
            ORDER BY 1, 2"
            .to_string(),
    };
    let rows = conn.query_rows(&sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let [schema, name, kind, parent, id]: [String; 5] = r.try_into().ok()?;
            let kind = match kind.as_str() {
                "U" => ObjectKind::Table,
                "V" => ObjectKind::View,
                "P" => ObjectKind::Procedure,
                "FN" | "IF" | "TF" => ObjectKind::Function,
                "TR" => ObjectKind::Trigger,
                _ => return None,
            };
            let parent = (parent != super::NULL && !parent.is_empty()).then_some(parent);
            Some(DbObject { kind, schema, name, parent, id })
        })
        .collect())
}

pub async fn columns(conn: Connection, database: String, object: DbObject) -> Result<Vec<ColumnInfo>, String> {
    let sql = match conn.driver() {
        Driver::SqlServer => format!(
            "SELECT c.name, \
               t.name + CASE \
                 WHEN t.name IN ('varchar','char','varbinary','binary') \
                   THEN '(' + CASE c.max_length WHEN -1 THEN 'max' ELSE CAST(c.max_length AS varchar(10)) END + ')' \
                 WHEN t.name IN ('nvarchar','nchar') \
                   THEN '(' + CASE c.max_length WHEN -1 THEN 'max' ELSE CAST(c.max_length / 2 AS varchar(10)) END + ')' \
                 WHEN t.name IN ('decimal','numeric') \
                   THEN '(' + CAST(c.precision AS varchar(3)) + ',' + CAST(c.scale AS varchar(3)) + ')' \
                 WHEN t.name IN ('datetime2','time','datetimeoffset') \
                   THEN '(' + CAST(c.scale AS varchar(3)) + ')' \
                 ELSE '' END, \
               c.is_nullable, \
               CASE WHEN EXISTS ( \
                 SELECT 1 FROM {db}.sys.index_columns ic \
                 JOIN {db}.sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
                 WHERE i.is_primary_key = 1 AND ic.object_id = c.object_id AND ic.column_id = c.column_id) \
               THEN 1 ELSE 0 END \
             FROM {db}.sys.columns c JOIN {db}.sys.types t ON t.user_type_id = c.user_type_id \
             WHERE c.object_id = {id} ORDER BY c.column_id",
            db = mssql_ident(&database),
            id = numeric_id(&object.id)?,
        ),
        Driver::Postgres => format!(
            "SELECT a.attname, format_type(a.atttypid, a.atttypmod), NOT a.attnotnull, \
               EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = a.attrelid AND i.indisprimary AND a.attnum = ANY(i.indkey)) \
             FROM pg_attribute a WHERE a.attrelid = {id} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
            id = numeric_id(&object.id)?,
        ),
    };
    let rows = conn.query_rows(&sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let [name, data_type, nullable, pk]: [String; 4] = r.try_into().ok()?;
            Some(ColumnInfo { name, data_type, nullable: is_true(&nullable), primary_key: is_true(&pk) })
        })
        .collect())
}

/// Everything autocomplete needs to know about one database.
#[derive(Debug, Clone, Default)]
pub struct SchemaInfo {
    /// Tables, views, procedures and functions (no triggers).
    pub objects: Vec<SchemaObject>,
}

#[derive(Debug, Clone)]
pub struct SchemaObject {
    pub kind: ObjectKind,
    pub schema: String,
    pub name: String,
    /// Name and data type, for tables and views.
    pub columns: Vec<(String, String)>,
}

/// Loads every object and every table/view column of a database in two queries.
pub async fn schema_info(conn: Connection, database: String) -> Result<SchemaInfo, String> {
    let driver = conn.driver();
    let objs = objects(conn.clone(), database.clone()).await?;
    let sql = match driver {
        Driver::SqlServer => format!(
            "SELECT s.name, o.name, c.name, t.name \
             FROM {db}.sys.columns c \
             JOIN {db}.sys.objects o ON o.object_id = c.object_id \
             JOIN {db}.sys.schemas s ON s.schema_id = o.schema_id \
             JOIN {db}.sys.types t ON t.user_type_id = c.user_type_id \
             WHERE o.is_ms_shipped = 0 AND o.type IN ('U','V') \
             ORDER BY s.name, o.name, c.column_id",
            db = mssql_ident(&database),
        ),
        Driver::Postgres => "\
            SELECT n.nspname, c.relname, a.attname, format_type(a.atttypid, a.atttypmod) \
            FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
            WHERE c.relkind IN ('r','p','v','m','f') AND a.attnum > 0 AND NOT a.attisdropped \
              AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' \
            ORDER BY 1, 2, a.attnum"
            .to_string(),
    };
    let mut columns: std::collections::HashMap<(String, String), Vec<(String, String)>> = Default::default();
    for row in conn.query_rows(&sql).await? {
        if let Ok([schema, table, column, data_type]) = <[String; 4]>::try_from(row) {
            columns.entry((schema, table)).or_default().push((column, data_type));
        }
    }
    let objects = objs
        .into_iter()
        .filter(|o| o.kind != ObjectKind::Trigger)
        .map(|o| {
            // Postgres routines are listed with their argument types: "name(int, text)".
            let name = match (driver, o.kind) {
                (Driver::Postgres, ObjectKind::Procedure | ObjectKind::Function) => {
                    o.name.split('(').next().unwrap_or_default().to_string()
                }
                _ => o.name,
            };
            let columns = columns.remove(&(o.schema.clone(), name.clone())).unwrap_or_default();
            SchemaObject { kind: o.kind, schema: o.schema, name, columns }
        })
        .collect();
    Ok(SchemaInfo { objects })
}

/// The CREATE script of a view, procedure, function or trigger.
pub async fn definition(conn: Connection, database: String, object: DbObject) -> Result<String, String> {
    let id = numeric_id(&object.id)?;
    let sql = match conn.driver() {
        Driver::SqlServer => {
            format!("SELECT OBJECT_DEFINITION({id}) FROM {}.sys.objects WHERE object_id = {id}", mssql_ident(&database))
        }
        Driver::Postgres => match object.kind {
            ObjectKind::View => format!(
                "SELECT 'CREATE OR REPLACE VIEW ' || {}::regclass::text || ' AS' || chr(10) || pg_get_viewdef({id}::oid, true)",
                id
            ),
            ObjectKind::Procedure | ObjectKind::Function => {
                format!("SELECT pg_get_functiondef({id}::oid)")
            }
            ObjectKind::Trigger => format!("SELECT pg_get_triggerdef({id}::oid, true) || ';'"),
            ObjectKind::Table => return Err("Tables have no definition to script.".into()),
        },
    };
    // OBJECT_DEFINITION needs the object's database to be current.
    let sql = match conn.driver() {
        Driver::SqlServer => format!("USE {};\n{sql}", mssql_ident(&database)),
        Driver::Postgres => sql,
    };
    let def = conn.query_rows(&sql).await?.into_iter().next().and_then(|r| r.into_iter().next());
    match def {
        Some(d) if d != super::NULL => Ok(d),
        _ => Err(format!("No definition available for {} (it may be encrypted).", object.qualified_name())),
    }
}

/// Script that opens in a new query tab for "Select Top 1000".
pub fn select_top_script(driver: Driver, object: &DbObject) -> String {
    match driver {
        Driver::SqlServer => {
            format!("SELECT TOP (1000) *\nFROM {}.{};\n", mssql_ident(&object.schema), mssql_ident(&object.name))
        }
        Driver::Postgres => {
            format!("SELECT *\nFROM {}.{}\nLIMIT 1000;\n", pg_ident(&object.schema), pg_ident(&object.name))
        }
    }
}

fn is_true(v: &str) -> bool {
    matches!(v, "1" | "t" | "true")
}

/// Ids come from our own catalog queries, but they are spliced into SQL, so check them.
fn numeric_id(id: &str) -> Result<&str, String> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit() || c == '-') {
        Ok(id)
    } else {
        Err(format!("Unexpected object id {id:?}"))
    }
}

pub fn mssql_ident(s: &str) -> String {
    format!("[{}]", s.replace(']', "]]"))
}

fn mssql_str(s: &str) -> String {
    format!("N'{}'", s.replace('\'', "''"))
}

fn pg_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(mssql_ident("a]b"), "[a]]b]");
        assert_eq!(mssql_str("o'k"), "N'o''k'");
        assert_eq!(pg_ident("my\"tbl"), "\"my\"\"tbl\"");
    }

    #[test]
    fn ids_are_validated() {
        assert!(numeric_id("12345").is_ok());
        assert!(numeric_id("1; DROP TABLE x").is_err());
        assert!(numeric_id("").is_err());
    }

    #[test]
    fn select_top() {
        let obj = DbObject {
            kind: ObjectKind::Table,
            schema: "dbo".into(),
            name: "Persons".into(),
            parent: None,
            id: "1".into(),
        };
        assert_eq!(select_top_script(Driver::SqlServer, &obj), "SELECT TOP (1000) *\nFROM [dbo].[Persons];\n");
        assert_eq!(select_top_script(Driver::Postgres, &obj), "SELECT *\nFROM \"dbo\".\"Persons\"\nLIMIT 1000;\n");
    }
}
