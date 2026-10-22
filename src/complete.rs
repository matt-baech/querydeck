//! SQL autocomplete. Works out what is being typed at the cursor (a keyword,
//! a table after `FROM`, a column after `alias.`, ...) and suggests matching
//! keywords, functions, databases, schemas, tables and columns.
//!
//! This is deliberately a light, token-based analysis rather than a parser:
//! it only needs to be right about the statement around the cursor, which is
//! usually incomplete anyway.

use crate::db::Driver;
use crate::db::catalog::{ObjectKind, SchemaInfo, SchemaObject};

/// At most this many suggestions are kept.
const MAX_ITEMS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Column,
    Alias,
    Table,
    View,
    Schema,
    Procedure,
    Function,
    Database,
    Keyword,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Shown in the list and matched against what was typed.
    pub label: String,
    /// Inserted on accept (quoted or schema-qualified where needed).
    pub insert: String,
    pub kind: Kind,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub items: Vec<Item>,
    pub selected: usize,
    /// How many characters before the cursor accepting replaces.
    pub replace: usize,
    /// Opened with Ctrl+Space, so it stays open even with nothing typed.
    pub forced: bool,
}

impl Completion {
    /// Moves the selection; single steps wrap around, page steps stop at the ends.
    pub fn select_next(&mut self, step: isize) {
        let n = self.items.len() as isize;
        if n == 0 {
            return;
        }
        let to = self.selected as isize + step;
        self.selected = if step.abs() == 1 { to.rem_euclid(n) } else { to.clamp(0, n - 1) } as usize;
    }
}

/// What autocomplete knows about the server.
pub struct Sources<'a> {
    pub driver: Driver,
    /// The tab's current database, if its metadata has been loaded.
    pub schema: Option<&'a SchemaInfo>,
    pub databases: &'a [String],
}

/// Suggestions for the cursor at byte offset `cursor` in `text`. Returns
/// `None` when there is nothing useful to show (inside a string or comment,
/// nothing typed yet unless `forced`, or no matches).
pub fn complete(text: &str, cursor: usize, sources: &Sources, forced: bool) -> Option<Completion> {
    let before = text.get(..cursor)?;
    let lexed = lex(before);
    let (prefix, quote, prefix_start) = match lexed.open {
        Open::String | Open::Comment => return None,
        Open::Quoted(q, start) => (&before[start + 1..], Some(q), start),
        Open::None => match lexed.tokens.last() {
            Some(t) if t.end == cursor && matches!(t.tok, Tok::Word(_)) => (&before[t.start..], None, t.start),
            Some(t) if t.end == cursor && matches!(t.tok, Tok::Quoted(_) | Tok::Other) => return None,
            _ => ("", None, cursor),
        },
    };
    if prefix.starts_with(|c: char| c.is_ascii_digit() || c == '@') {
        return None;
    }

    // Tokens before the word being typed, back to the start of the statement.
    let mut ctx: Vec<&Token> = lexed.tokens.iter().filter(|t| t.end <= prefix_start).collect();
    if let Some(i) = ctx.iter().rposition(|t| t.is_boundary()) {
        ctx.drain(..=i);
    }
    let qualifier = take_qualifier(&mut ctx);
    if !forced && prefix.is_empty() && qualifier.is_empty() {
        return None;
    }

    let keywords = Keywords::for_driver(sources.driver);
    let refs = statement_refs(text, cursor, &keywords);
    let context = context(&ctx, &keywords);
    let mut out = Builder { prefix, quote, driver: sources.driver, keywords: &keywords, items: Vec::new() };

    if qualifier.is_empty() {
        unqualified(&mut out, context, sources, &refs);
    } else {
        qualified(&mut out, context, sources, &refs, &qualifier);
    }

    let mut items = out.items;
    items.sort_by_cached_key(|(rank, item)| (*rank, item.label.to_lowercase()));
    items.dedup_by(|(_, a), (_, b)| a.kind == b.kind && a.label.eq_ignore_ascii_case(&b.label));
    let items: Vec<Item> = items.into_iter().map(|(_, item)| item).take(MAX_ITEMS).collect();

    // Don't keep a popup open that only repeats what is already typed.
    let only_echo = items.len() == 1 && items[0].label.eq_ignore_ascii_case(prefix);
    if items.is_empty() || (only_echo && !forced) {
        return None;
    }
    let replace = before[prefix_start..].chars().count();
    Some(Completion { items, selected: 0, replace, forced })
}

// ---------------------------------------------------------------- contexts

/// What kind of thing belongs at the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    /// Start of a statement: keywords.
    Start,
    /// After FROM, JOIN, UPDATE, INTO ...: tables and views.
    Relation,
    /// After a table name in a FROM clause: an alias or the next keyword.
    AfterRelation,
    /// After EXEC / CALL.
    Procedure,
    /// After USE.
    Database,
    /// Anywhere else: columns, functions and keywords.
    Expression,
}

const RELATION_KEYWORDS: [&str; 5] = ["FROM", "JOIN", "UPDATE", "INTO", "TABLE"];

fn context(ctx: &[&Token], keywords: &Keywords) -> Context {
    let Some(prev) = ctx.last() else {
        return Context::Start;
    };
    if let Some(word) = prev.keyword(keywords) {
        return match word.as_str() {
            w if RELATION_KEYWORDS.contains(&w) => Context::Relation,
            "EXEC" | "EXECUTE" | "CALL" => Context::Procedure,
            "USE" => Context::Database,
            _ => Context::Expression,
        };
    }
    // Walk back over names, aliases and commas to the keyword that owns them.
    let owner = ctx
        .iter()
        .rev()
        .find(|t| match &t.tok {
            Tok::Word(w) => keywords.contains(w) && !w.eq_ignore_ascii_case("AS"),
            Tok::Quoted(_) => false,
            Tok::Punct(c) => !matches!(c, '.' | ','),
            Tok::Other => true,
        })
        .and_then(|t| t.keyword(keywords));
    let in_from = owner.as_deref().is_some_and(|w| RELATION_KEYWORDS.contains(&w));
    match &prev.tok {
        Tok::Punct(',') if owner.as_deref() == Some("FROM") => Context::Relation,
        Tok::Word(_) | Tok::Quoted(_) if in_from => Context::AfterRelation,
        _ => Context::Expression,
    }
}

/// Removes a trailing `name.` / `schema.table.` from `ctx` and returns its parts.
fn take_qualifier(ctx: &mut Vec<&Token>) -> Vec<String> {
    let mut parts = Vec::new();
    while matches!(ctx.last().map(|t| &t.tok), Some(Tok::Punct('.'))) {
        let dot = ctx.len() - 1;
        match ctx.get(dot.wrapping_sub(1)).and_then(|t| t.name()) {
            Some(name) => {
                parts.insert(0, name.to_string());
                ctx.truncate(dot - 1);
            }
            None => {
                ctx.truncate(dot);
                break;
            }
        }
    }
    parts
}

// ---------------------------------------------------------------- suggestions

struct Builder<'a> {
    prefix: &'a str,
    /// The quote the user opened (`[` or `"`), if any.
    quote: Option<char>,
    driver: Driver,
    keywords: &'a Keywords,
    items: Vec<(u8, Item)>,
}

impl Builder<'_> {
    fn matches(&self, label: &str) -> bool {
        let n = self.prefix.len();
        label.len() >= n && label.is_char_boundary(n) && label[..n].eq_ignore_ascii_case(self.prefix)
    }

    fn push(&mut self, rank: u8, kind: Kind, label: &str, insert: String, detail: String) {
        if self.matches(label) {
            self.items.push((rank, Item { label: label.to_string(), insert, kind, detail }));
        }
    }

    fn keyword(&mut self, rank: u8, kind: Kind, word: &str) {
        if self.quote.is_some() {
            return;
        }
        // Follow the case the user is typing in.
        let lower = !self.prefix.is_empty() && !self.prefix.chars().any(|c| c.is_ascii_uppercase());
        let insert = if lower { word.to_ascii_lowercase() } else { word.to_string() };
        let detail = if kind == Kind::Function { "function" } else { "keyword" };
        self.push(rank, kind, word, insert, detail.into());
    }

    fn name(&mut self, rank: u8, kind: Kind, name: &str, detail: String) {
        let insert = self.ident(name);
        self.push(rank, kind, name, insert, detail);
    }

    /// An object, schema-qualified unless it is in the default schema.
    fn object(&mut self, rank: u8, obj: &SchemaObject, qualify: bool) {
        let kind = match obj.kind {
            ObjectKind::Table => Kind::Table,
            ObjectKind::View => Kind::View,
            ObjectKind::Procedure => Kind::Procedure,
            ObjectKind::Function | ObjectKind::Trigger => Kind::Function,
        };
        let insert = if qualify && !is_default_schema(self.driver, &obj.schema) && self.quote.is_none() {
            format!("{}.{}", self.ident(&obj.schema), self.ident(&obj.name))
        } else {
            self.ident(&obj.name)
        };
        let detail = format!("{} · {}", kind_name(kind), obj.schema);
        self.push(rank, kind, &obj.name, insert, detail);
    }

    fn columns(&mut self, rank: u8, obj: &SchemaObject, show_table: bool) {
        for (name, data_type) in &obj.columns {
            let detail = if show_table { format!("{data_type} · {}", obj.name) } else { data_type.clone() };
            self.name(rank, Kind::Column, name, detail);
        }
    }

    /// `name` as it must be written in SQL: quoted if the user opened a
    /// quote or the name is not a plain identifier.
    fn ident(&self, name: &str) -> String {
        let (open, close) = match (self.quote, self.driver) {
            (Some('['), _) | (None, Driver::SqlServer) => ('[', ']'),
            _ => ('"', '"'),
        };
        let plain = match self.driver {
            Driver::SqlServer => {
                name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '#')
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '#' | '$' | '@'))
            }
            Driver::Postgres => {
                name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
                    && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '$'))
            }
        };
        if self.quote.is_none() && plain && !self.keywords.contains(name) {
            return name.to_string();
        }
        let escaped = name.replace(close, &format!("{close}{close}"));
        format!("{open}{escaped}{close}")
    }
}

fn unqualified(out: &mut Builder, context: Context, sources: &Sources, refs: &[TableRef]) {
    let keywords = out.keywords;
    let objects = sources.schema.map(|s| s.objects.as_slice()).unwrap_or_default();
    match context {
        Context::Start | Context::AfterRelation => {
            for k in keywords.words {
                out.keyword(0, Kind::Keyword, k);
            }
        }
        Context::Relation => {
            for obj in objects.iter().filter(|o| matches!(o.kind, ObjectKind::Table | ObjectKind::View)) {
                out.object(0, obj, true);
            }
            for schema in schemas(objects) {
                out.name(1, Kind::Schema, schema, "schema".into());
            }
        }
        Context::Procedure => {
            for obj in objects.iter().filter(|o| o.kind == ObjectKind::Procedure) {
                out.object(0, obj, true);
            }
            for schema in schemas(objects) {
                out.name(1, Kind::Schema, schema, "schema".into());
            }
        }
        Context::Database => {
            for db in sources.databases {
                out.name(0, Kind::Database, db, "database".into());
            }
        }
        Context::Expression => {
            let resolved: Vec<(&TableRef, &SchemaObject)> =
                refs.iter().filter_map(|r| Some((r, resolve(objects, &r.parts)?))).collect();
            let several = resolved.len() > 1;
            for (r, obj) in &resolved {
                out.columns(0, obj, several);
                if let Some(alias) = &r.alias {
                    out.name(1, Kind::Alias, alias, format!("alias · {}", obj.name));
                }
            }
            for k in keywords.words {
                out.keyword(2, Kind::Keyword, k);
            }
            for f in keywords.functions {
                out.keyword(3, Kind::Function, f);
            }
            for obj in objects.iter().filter(|o| matches!(o.kind, ObjectKind::Table | ObjectKind::View)) {
                out.object(4, obj, true);
            }
        }
    }
}

fn qualified(out: &mut Builder, context: Context, sources: &Sources, refs: &[TableRef], qualifier: &[String]) {
    let objects = sources.schema.map(|s| s.objects.as_slice()).unwrap_or_default();
    let last = qualifier.last().map(String::as_str).unwrap_or_default();

    // alias. / table. / schema.table.  → that table's columns
    if !matches!(context, Context::Relation | Context::Procedure) {
        let table = refs
            .iter()
            .find(|r| r.alias.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(last)))
            .and_then(|r| resolve(objects, &r.parts))
            .or_else(|| resolve(objects, qualifier));
        if let Some(obj) = table {
            out.columns(0, obj, false);
            return;
        }
    }

    // schema.  → objects in that schema
    let in_schema = objects.iter().filter(|o| o.schema.eq_ignore_ascii_case(last));
    for obj in in_schema {
        let wanted = match context {
            Context::Relation => matches!(obj.kind, ObjectKind::Table | ObjectKind::View),
            Context::Procedure => obj.kind == ObjectKind::Procedure,
            _ => obj.kind != ObjectKind::Procedure,
        };
        if wanted {
            out.object(0, obj, false);
        }
    }
}

/// Finds the object `parts` (`[table]` or `[..., schema, table]`) names,
/// preferring the default schema when only a table name is given.
fn resolve<'a>(objects: &'a [SchemaObject], parts: &[String]) -> Option<&'a SchemaObject> {
    let (schema, name) = match parts {
        [] => return None,
        [name] => (None, name),
        [.., schema, name] => (Some(schema), name),
    };
    let mut candidates = objects
        .iter()
        .filter(|o| matches!(o.kind, ObjectKind::Table | ObjectKind::View))
        .filter(|o| o.name.eq_ignore_ascii_case(name))
        .filter(|o| schema.is_none_or(|s| o.schema.eq_ignore_ascii_case(s)));
    let first = candidates.next()?;
    Some(
        std::iter::once(first)
            .chain(candidates)
            .find(|o| matches!(o.schema.as_str(), "dbo" | "public"))
            .unwrap_or(first),
    )
}

fn schemas(objects: &[SchemaObject]) -> Vec<&str> {
    let mut s: Vec<&str> = objects.iter().map(|o| o.schema.as_str()).collect();
    s.sort_unstable();
    s.dedup();
    s
}

fn is_default_schema(driver: Driver, schema: &str) -> bool {
    match driver {
        Driver::SqlServer => schema.eq_ignore_ascii_case("dbo"),
        Driver::Postgres => schema == "public",
    }
}

pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Column => "column",
        Kind::Alias => "alias",
        Kind::Table => "table",
        Kind::View => "view",
        Kind::Schema => "schema",
        Kind::Procedure => "procedure",
        Kind::Function => "function",
        Kind::Database => "database",
        Kind::Keyword => "keyword",
    }
}

// ---------------------------------------------------------------- table references

/// A table named in the statement, e.g. `dbo.Persons AS p`.
#[derive(Debug, Clone, PartialEq)]
struct TableRef {
    parts: Vec<String>,
    alias: Option<String>,
}

/// Tables referenced by the statement around the cursor (the whole
/// statement, since the FROM clause usually comes after the SELECT list).
fn statement_refs(text: &str, cursor: usize, keywords: &Keywords) -> Vec<TableRef> {
    let tokens = lex(text).tokens;
    let start = tokens.iter().rposition(|t| t.end <= cursor && t.is_boundary()).map_or(0, |i| i + 1);
    let end = tokens.iter().position(|t| t.start >= cursor && t.is_boundary()).unwrap_or(tokens.len());
    let tokens = &tokens[start..end.max(start)];

    let mut refs = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let opens_list = tokens[i].keyword(keywords).is_some_and(|w| RELATION_KEYWORDS.contains(&w.as_str()));
        i += 1;
        if !opens_list {
            continue;
        }
        loop {
            let mut parts = Vec::new();
            while let Some(name) = tokens.get(i).and_then(|t| t.name()) {
                if tokens[i].keyword(keywords).is_some() {
                    break;
                }
                parts.push(name.to_string());
                i += 1;
                if matches!(tokens.get(i).map(|t| &t.tok), Some(Tok::Punct('.'))) {
                    i += 1;
                } else {
                    break;
                }
            }
            if parts.is_empty() {
                break;
            }
            if tokens.get(i).and_then(|t| t.keyword(keywords)).as_deref() == Some("AS") {
                i += 1;
            }
            let alias = match tokens.get(i) {
                Some(t) if t.keyword(keywords).is_none() => t.name().map(str::to_string),
                _ => None,
            };
            if alias.is_some() {
                i += 1;
            }
            refs.push(TableRef { parts, alias });
            if matches!(tokens.get(i).map(|t| &t.tok), Some(Tok::Punct(','))) {
                i += 1;
            } else {
                break;
            }
        }
    }
    refs
}

// ---------------------------------------------------------------- lexer

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// An unquoted word: keyword, identifier or variable.
    Word(String),
    /// `[name]` or `"name"`, without the quotes.
    Quoted(String),
    Punct(char),
    /// String or number literal.
    Other,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    start: usize,
    end: usize,
}

impl Token {
    /// The upper-cased keyword, if this token is one.
    fn keyword(&self, keywords: &Keywords) -> Option<String> {
        match &self.tok {
            Tok::Word(w) if keywords.contains(w) => Some(w.to_ascii_uppercase()),
            _ => None,
        }
    }

    fn name(&self) -> Option<&str> {
        match &self.tok {
            Tok::Word(w) | Tok::Quoted(w) => Some(w),
            _ => None,
        }
    }

    /// `;` or a `GO` batch separator ends a statement.
    fn is_boundary(&self) -> bool {
        match &self.tok {
            Tok::Punct(';') => true,
            Tok::Word(w) => w.eq_ignore_ascii_case("GO"),
            _ => false,
        }
    }
}

/// An unfinished token at the end of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Open {
    None,
    String,
    Comment,
    /// Opening quote and its byte offset.
    Quoted(char, usize),
}

struct Lexed {
    tokens: Vec<Token>,
    open: Open,
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '@' | '#' | '$')
}

fn lex(text: &str) -> Lexed {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut open = Open::None;
    let push = |tokens: &mut Vec<Token>, tok, start, end| tokens.push(Token { tok, start, end });

    while let Some((start, c)) = chars.next() {
        open = Open::None;
        match c {
            c if c.is_whitespace() => {}
            '-' if chars.peek().is_some_and(|&(_, n)| n == '-') => {
                open = Open::Comment;
                for (_, n) in chars.by_ref() {
                    if n == '\n' {
                        open = Open::None;
                        break;
                    }
                }
            }
            '/' if chars.peek().is_some_and(|&(_, n)| n == '*') => {
                chars.next();
                open = Open::Comment;
                let mut star = false;
                for (_, n) in chars.by_ref() {
                    if star && n == '/' {
                        open = Open::None;
                        break;
                    }
                    star = n == '*';
                }
            }
            '\'' => {
                open = Open::String;
                while let Some((_, n)) = chars.next() {
                    if n == '\'' {
                        if chars.peek().is_some_and(|&(_, n)| n == '\'') {
                            chars.next();
                        } else {
                            open = Open::None;
                            break;
                        }
                    }
                }
                let end = chars.peek().map_or(text.len(), |&(i, _)| i);
                push(&mut tokens, Tok::Other, start, end);
            }
            '[' | '"' => {
                let close = if c == '[' { ']' } else { '"' };
                open = Open::Quoted(c, start);
                let mut name = String::new();
                while let Some((_, n)) = chars.next() {
                    if n == close {
                        if chars.peek().is_some_and(|&(_, n)| n == close) {
                            chars.next();
                            name.push(n);
                        } else {
                            open = Open::None;
                            break;
                        }
                    } else {
                        name.push(n);
                    }
                }
                if open == Open::None {
                    let end = chars.peek().map_or(text.len(), |&(i, _)| i);
                    push(&mut tokens, Tok::Quoted(name), start, end);
                }
            }
            c if c.is_ascii_digit() => {
                while chars.peek().is_some_and(|&(_, n)| n.is_ascii_alphanumeric() || n == '.') {
                    chars.next();
                }
                let end = chars.peek().map_or(text.len(), |&(i, _)| i);
                push(&mut tokens, Tok::Other, start, end);
            }
            c if is_word_char(c) => {
                // N'unicode string'
                if matches!(c, 'N' | 'n') && chars.peek().is_some_and(|&(_, n)| n == '\'') {
                    continue;
                }
                while chars.peek().is_some_and(|&(_, n)| is_word_char(n)) {
                    chars.next();
                }
                let end = chars.peek().map_or(text.len(), |&(i, _)| i);
                push(&mut tokens, Tok::Word(text[start..end].to_string()), start, end);
            }
            c => push(&mut tokens, Tok::Punct(c), start, start + c.len_utf8()),
        }
    }
    Lexed { tokens, open }
}

// ---------------------------------------------------------------- word lists

struct Keywords {
    words: &'static [&'static str],
    functions: &'static [&'static str],
}

impl Keywords {
    fn for_driver(driver: Driver) -> Self {
        match driver {
            Driver::SqlServer => Keywords { words: MSSQL_KEYWORDS, functions: MSSQL_FUNCTIONS },
            Driver::Postgres => Keywords { words: PG_KEYWORDS, functions: PG_FUNCTIONS },
        }
    }

    fn contains(&self, word: &str) -> bool {
        self.words.iter().any(|k| k.eq_ignore_ascii_case(word))
    }
}

macro_rules! common_keywords {
    ($($extra:literal),* $(,)?) => {
        &[
            "ADD", "ALL", "ALTER", "AND", "ANY", "AS", "ASC", "BEGIN", "BETWEEN", "BY", "CASCADE", "CASE",
            "CHECK", "COLUMN", "COMMIT", "CONSTRAINT", "CREATE", "CROSS", "CURRENT_DATE", "CURRENT_TIMESTAMP",
            "DATABASE", "DECLARE", "DEFAULT", "DELETE", "DESC", "DISTINCT", "DROP", "ELSE", "END", "EXCEPT",
            "EXISTS", "FOREIGN", "FROM", "FULL", "FUNCTION", "GRANT", "GROUP", "HAVING", "IF", "IN", "INDEX",
            "INNER", "INSERT", "INTERSECT", "INTO", "IS", "JOIN", "KEY", "LEFT", "LIKE", "NOT", "NULL", "ON",
            "OR", "ORDER", "OUTER", "OVER", "PARTITION", "PRIMARY", "PROCEDURE", "REFERENCES", "RETURNS",
            "REVOKE", "RIGHT", "ROLLBACK", "SCHEMA", "SELECT", "SET", "TABLE", "THEN", "TO", "TRANSACTION",
            "TRIGGER", "TRUNCATE", "UNION", "UNIQUE", "UPDATE", "VALUES", "VIEW", "WHEN", "WHERE", "WITH",
            $($extra),*
        ]
    };
}

const MSSQL_KEYWORDS: &[&str] = common_keywords!(
    "APPLY",
    "CATCH",
    "CLUSTERED",
    "EXEC",
    "EXECUTE",
    "FETCH",
    "GO",
    "IDENTITY",
    "MATCHED",
    "MERGE",
    "NEXT",
    "NOCOUNT",
    "NOLOCK",
    "NONCLUSTERED",
    "OFFSET",
    "ONLY",
    "OPTION",
    "OUTPUT",
    "PERCENT",
    "PIVOT",
    "PRINT",
    "PROC",
    "RAISERROR",
    "RETURN",
    "ROWS",
    "THROW",
    "TIES",
    "TOP",
    "TRAN",
    "TRY",
    "UNPIVOT",
    "USE",
    "WHILE",
);

const PG_KEYWORDS: &[&str] = common_keywords!(
    "CALL",
    "CONFLICT",
    "DO",
    "EXTENSION",
    "FETCH",
    "FILTER",
    "FIRST",
    "ILIKE",
    "LANGUAGE",
    "LATERAL",
    "LIMIT",
    "MATERIALIZED",
    "NOTHING",
    "NULLS",
    "OFFSET",
    "ONLY",
    "RECURSIVE",
    "REPLACE",
    "RETURNING",
    "ROWS",
    "SEQUENCE",
    "SERIAL",
    "USING",
    "WINDOW",
);

macro_rules! common_functions {
    ($($extra:literal),* $(,)?) => {
        &[
            "ABS", "AVG", "CAST", "CEILING", "COALESCE", "CONCAT", "COUNT", "DENSE_RANK", "FIRST_VALUE",
            "FLOOR", "LAG", "LAST_VALUE", "LEAD", "LOWER", "LTRIM", "MAX", "MIN", "NTILE", "NULLIF", "RANK",
            "REPLACE", "ROUND", "ROW_NUMBER", "RTRIM", "STRING_AGG", "SUBSTRING", "SUM", "TRIM", "UPPER",
            $($extra),*
        ]
    };
}

const MSSQL_FUNCTIONS: &[&str] = common_functions!(
    "CHARINDEX",
    "CONVERT",
    "DATEADD",
    "DATEDIFF",
    "DATENAME",
    "DATEPART",
    "DAY",
    "DB_NAME",
    "EOMONTH",
    "FORMAT",
    "GETDATE",
    "GETUTCDATE",
    "IIF",
    "ISNULL",
    "LEN",
    "MONTH",
    "NEWID",
    "OBJECT_ID",
    "SCOPE_IDENTITY",
    "STRING_SPLIT",
    "SYSDATETIME",
    "TRY_CAST",
    "TRY_CONVERT",
    "YEAR",
);

const PG_FUNCTIONS: &[&str] = common_functions!(
    "AGE",
    "ARRAY_AGG",
    "DATE_PART",
    "DATE_TRUNC",
    "EXTRACT",
    "GEN_RANDOM_UUID",
    "GENERATE_SERIES",
    "JSON_AGG",
    "JSONB_BUILD_OBJECT",
    "LENGTH",
    "NOW",
    "POSITION",
    "SPLIT_PART",
    "TO_CHAR",
    "TO_DATE",
    "TO_TIMESTAMP",
);

#[cfg(test)]
mod tests {
    use super::*;

    fn persons() -> SchemaInfo {
        let obj = |kind, schema: &str, name: &str, cols: &[&str]| SchemaObject {
            kind,
            schema: schema.into(),
            name: name.into(),
            columns: cols.iter().map(|c| (c.to_string(), "int".to_string())).collect(),
        };
        SchemaInfo {
            objects: vec![
                obj(ObjectKind::Table, "dbo", "Persons", &["PersonId", "FirstName", "LastName", "BirthDate"]),
                obj(ObjectKind::Table, "dbo", "Orders", &["OrderId", "PersonId", "Total"]),
                obj(ObjectKind::Table, "sales", "Invoices", &["InvoiceId", "Amount"]),
                obj(ObjectKind::Table, "dbo", "Order Lines", &["LineId"]),
                obj(ObjectKind::Procedure, "dbo", "GetPerson", &[]),
            ],
        }
    }

    /// Completes at the `|` marker and returns the labels.
    fn labels(driver: Driver, sql: &str, forced: bool) -> Vec<String> {
        let schema = persons();
        let dbs = vec!["master".to_string(), "test".to_string()];
        let sources = Sources { driver, schema: Some(&schema), databases: &dbs };
        let cursor = sql.find('|').unwrap();
        let text = sql.replace('|', "");
        complete(&text, cursor, &sources, forced)
            .map(|c| c.items.into_iter().map(|i| i.label).collect())
            .unwrap_or_default()
    }

    fn first(driver: Driver, sql: &str) -> Option<Item> {
        let schema = persons();
        let sources = Sources { driver, schema: Some(&schema), databases: &[] };
        let cursor = sql.find('|').unwrap();
        complete(&sql.replace('|', ""), cursor, &sources, false).and_then(|c| c.items.into_iter().next())
    }

    #[test]
    fn keywords_at_statement_start() {
        assert_eq!(labels(Driver::SqlServer, "sel|", false), vec!["SELECT"]);
        assert_eq!(first(Driver::SqlServer, "sel|").unwrap().insert, "select");
        assert_eq!(first(Driver::SqlServer, "SEL|").unwrap().insert, "SELECT");
    }

    #[test]
    fn tables_after_from() {
        let l = labels(Driver::SqlServer, "SELECT * FROM P|", false);
        assert_eq!(l, vec!["Persons"]);
        let l = labels(Driver::SqlServer, "SELECT * FROM |", true);
        assert!(l.contains(&"Orders".to_string()) && l.contains(&"sales".to_string()));
        assert!(!l.contains(&"GetPerson".to_string()));
    }

    #[test]
    fn non_default_schema_is_qualified_and_odd_names_quoted() {
        assert_eq!(first(Driver::SqlServer, "SELECT * FROM Inv|").unwrap().insert, "sales.Invoices");
        assert_eq!(first(Driver::SqlServer, "SELECT * FROM [Order L|").unwrap().insert, "[Order Lines]");
        assert_eq!(first(Driver::Postgres, "SELECT * FROM Per|").unwrap().insert, "dbo.\"Persons\"");
    }

    #[test]
    fn columns_from_alias_and_table() {
        let l = labels(Driver::SqlServer, "SELECT p.| FROM dbo.Persons p", false);
        assert_eq!(l, vec!["BirthDate", "FirstName", "LastName", "PersonId"]);
        let l =
            labels(Driver::SqlServer, "SELECT o.T| FROM Persons AS p JOIN Orders o ON o.PersonId = p.PersonId", false);
        assert_eq!(l, vec!["Total"]);
        let l = labels(Driver::SqlServer, "SELECT Persons.L| FROM Persons", false);
        assert_eq!(l, vec!["LastName"]);
    }

    #[test]
    fn unqualified_columns_come_first() {
        let l = labels(Driver::SqlServer, "SELECT * FROM Persons WHERE Fi|", false);
        assert_eq!(l[0], "FirstName");
        let l = labels(Driver::SqlServer, "SELECT P| FROM Orders", false);
        assert_eq!(l[0], "PersonId");
        assert!(l.contains(&"PERCENT".to_string()));
    }

    #[test]
    fn schema_qualifier_lists_its_objects() {
        assert_eq!(labels(Driver::SqlServer, "SELECT * FROM sales.|", false), vec!["Invoices"]);
        assert_eq!(first(Driver::SqlServer, "SELECT * FROM sales.I|").unwrap().insert, "Invoices");
    }

    #[test]
    fn statement_boundaries_limit_references() {
        let l = labels(Driver::SqlServer, "SELECT * FROM Orders;\nSELECT Fi| FROM Persons", false);
        assert_eq!(l[0], "FirstName");
        let l = labels(Driver::SqlServer, "SELECT * FROM Orders\nGO\nSELECT Tot|", false);
        assert!(l.is_empty());
    }

    #[test]
    fn procedures_and_databases() {
        assert_eq!(labels(Driver::SqlServer, "EXEC Get|", false), vec!["GetPerson"]);
        assert_eq!(labels(Driver::SqlServer, "USE te|", false), vec!["test"]);
    }

    #[test]
    fn nothing_inside_strings_comments_or_numbers() {
        assert!(labels(Driver::SqlServer, "SELECT 'Fro|", false).is_empty());
        assert!(labels(Driver::SqlServer, "-- sel|", false).is_empty());
        assert!(labels(Driver::SqlServer, "/* sel|", false).is_empty());
        assert!(labels(Driver::SqlServer, "SELECT 12|", false).is_empty());
        assert!(!labels(Driver::SqlServer, "/* x */ sel|", false).is_empty());
    }

    #[test]
    fn after_a_table_name_suggests_keywords() {
        let l = labels(Driver::SqlServer, "SELECT * FROM Persons W|", false);
        assert_eq!(l, vec!["WHEN", "WHERE", "WHILE", "WITH"]);
    }

    #[test]
    fn needs_typing_unless_forced() {
        assert!(labels(Driver::SqlServer, "SELECT * FROM |", false).is_empty());
        assert!(labels(Driver::SqlServer, "SELECT * FROM Persons|", false).is_empty());
    }

    #[test]
    fn replace_counts_the_open_quote() {
        let schema = persons();
        let sources = Sources { driver: Driver::SqlServer, schema: Some(&schema), databases: &[] };
        let text = "SELECT * FROM [Per";
        assert_eq!(complete(text, text.len(), &sources, false).unwrap().replace, 4);
    }
}
