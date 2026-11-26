use crate::{index::Index, transaction::Plan};
use anyhow::{Context, Result, bail, ensure};
use mach_codegen::{Entity, Field, quoted as q};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

type Schema = BTreeMap<String, Entity>;
const SNAPSHOT: &str = "-- mach-schema-v1 ";
fn history() -> Result<(u64, Schema)> {
    let mut last = (0, Schema::new());
    if !Path::new("migrations").exists() {
        return Ok(last);
    }
    let mut versions = BTreeSet::new();
    for entry in fs::read_dir("migrations")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|s| s != "sql") {
            continue;
        }
        crate::transaction::checked(&path)?;
        let name = entry.file_name();
        let name = name.to_str().context("migration filename")?;
        let (prefix, _) = name.split_once('_').context("expected version_name.sql")?;
        let version: u64 = prefix.parse()?;
        ensure!(versions.insert(version), "duplicate migration version");
        if version > last.0 {
            last.0 = version
        }
    }
    let mut checkpoint = 0;
    for entry in fs::read_dir("migrations")? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with("_mach.sql") {
            continue;
        }
        let version: u64 = name.split_once('_').unwrap().0.parse()?;
        if version <= checkpoint {
            continue;
        }
        let content = fs::read_to_string(path)?;
        last.1 = serde_json::from_str(
            content
                .lines()
                .last()
                .and_then(|s| s.strip_prefix(SNAPSHOT))
                .context("managed migration is missing its schema snapshot")?,
        )?;
        checkpoint = version;
    }
    Ok(last)
}
fn object(table: &str, key: &str, kind: &str) -> String {
    let digest = Sha256::digest(format!("{table}/{key}/{kind}"));
    q(&format!("mach_{kind}_{:.24}", format!("{digest:x}")))
}
fn decorations(out: &mut Vec<String>, table: &str, f: &Field, old: Option<&Field>) {
    let t = q(table);
    let c = q(&f.name);
    for (kind, bound, prev, op) in [
        ("min", f.min, old.and_then(|f| f.min), ">="),
        ("max", f.max, old.and_then(|f| f.max), "<="),
    ] {
        if bound == prev {
            continue;
        }
        let constraint = object(table, &f.key, kind);
        if prev.is_some() {
            out.push(format!("ALTER TABLE {t} DROP CONSTRAINT {constraint};"))
        }
        if let Some(n) = bound {
            out.push(format!(
                "ALTER TABLE {t} ADD CONSTRAINT {constraint} CHECK (char_length({c}) {op} {n});"
            ))
        }
    }
    let old_index = old
        .map(|f| (f.unique, f.index || f.unique))
        .unwrap_or_default();
    if old_index != (f.unique, f.unique || f.index) {
        let name = object(table, &f.key, "idx");
        if old_index.1 {
            out.push(format!("DROP INDEX {name};"))
        }
        if f.index || f.unique {
            let unique = if f.unique { "UNIQUE " } else { "" };
            out.push(format!("CREATE {unique}INDEX {name} ON {t} ({c});"))
        }
    }
}
pub struct Migration {
    pub path: Option<String>,
    pub sql: String,
    pub destructive: bool,
    pub review: Vec<String>,
}
impl Migration {
    pub fn plan(self, allow: bool) -> Result<Plan> {
        ensure!(
            !self.destructive || allow,
            "destructive schema change requires --allow-destructive"
        );
        let mut plan = Plan::default();
        if let Some(p) = self.path {
            plan.create(&p, self.sql)?
        }
        Ok(plan)
    }
}
fn target<'a>(schema: &'a Schema, reference: &str) -> Option<(&'a Entity, &'a Field)> {
    let (table, column) = reference.split_once('.')?;
    let entity = schema.get(table)?;
    Some((entity, entity.fields.iter().find(|f| f.name == column)?))
}
fn same_reference(old: &Schema, new: &Schema, a: &Field, b: &Field) -> bool {
    match (a.references.as_deref(), b.references.as_deref()) {
        (None, None) => true,
        (Some(a), Some(b)) => target(old, a)
            .zip(target(new, b))
            .is_some_and(|((x, f), (y, g))| x.table == y.table && f.key == g.key),
        _ => false,
    }
}
pub fn diff(index: &Index) -> Result<Migration> {
    let (version, old) = history()?;
    let new = index.schema();
    let mut out = Vec::new();
    let mut destructive = false;
    let mut review = Vec::new();
    let mut new_tables = BTreeSet::new();
    let mut foreign_keys = Vec::new();
    for (t, e) in &new {
        if old.contains_key(t) {
            continue;
        }
        new_tables.insert(t);
        let columns = e
            .fields
            .iter()
            .map(|f| {
                let mut f = f.clone();
                f.references = None;
                f.column()
            })
            .collect::<Vec<_>>();
        out.push(format!(
            "CREATE TABLE {} (\n    {}\n);",
            q(t),
            columns.join(",\n    ")
        ));
    }
    for (t, e) in &new {
        let table = q(t);
        let existing = old.get(t);
        for f in &e.fields {
            let prev = existing.and_then(|e| e.fields.iter().find(|p| p.key == f.key));
            if let Some(reference) = &f.references {
                let (entity, column) = target(&new, reference)
                    .context("foreign key points to an unknown table or column")?;
                ensure!(
                    (column.id || column.unique) && column.kind == f.kind,
                    "invalid foreign key from {t}.{} to {reference}",
                    f.name
                );
                if prev.is_none() {
                    foreign_keys.push(format!(
                        "ALTER TABLE {table} ADD FOREIGN KEY ({}) REFERENCES {} ({});",
                        q(&f.name),
                        q(&entity.table),
                        q(&column.name)
                    ));
                }
            }
            if let Some(prev) = prev {
                ensure!(
                    prev.kind == f.kind
                        && prev.id == f.id
                        && prev.optional == f.optional
                        && prev.default == f.default
                        && same_reference(&old, &new, prev, f),
                    "column {}.{} requires a hand-reviewed migration",
                    t,
                    f.name
                );
                if prev.name != f.name {
                    out.push(format!(
                        "ALTER TABLE {table} RENAME COLUMN {} TO {};",
                        q(&prev.name),
                        q(&f.name)
                    ))
                }
            } else if !new_tables.contains(t) {
                let mut column = f.clone();
                column.references = None;
                if !f.optional && f.default.is_none() {
                    let note = format!(
                        "{t}.{} is required without a default; backfill existing rows before applying",
                        f.name
                    );
                    out.push(format!("-- mach-review: {note}"));
                    review.push(note);
                }
                out.push(format!(
                    "ALTER TABLE {table} ADD COLUMN {};",
                    column.column()
                ))
            }
            decorations(&mut out, t, f, prev);
        }
        if let Some(old) = existing {
            for f in &old.fields {
                if !e.fields.iter().any(|n| n.key == f.key) {
                    destructive = true;
                    out.push(format!("ALTER TABLE {table} DROP COLUMN {};", q(&f.name)))
                }
            }
        }
    }
    out.extend(foreign_keys);
    let dropped = old
        .keys()
        .filter(|t| !new.contains_key(*t))
        .map(|t| q(t))
        .collect::<Vec<_>>();
    if !dropped.is_empty() {
        destructive = true;
        out.push(format!("DROP TABLE {};", dropped.join(", ")))
    }
    if out.is_empty() {
        return Ok(Migration {
            path: None,
            sql: String::new(),
            destructive: false,
            review,
        });
    }
    if destructive {
        out.insert(0, "-- mach-destructive".into())
    }
    out.push(format!("{SNAPSHOT}{}", serde_json::to_string(&new)?));
    Ok(Migration {
        path: Some(format!(
            "migrations/{:06}_mach.sql",
            version.checked_add(1).context("version overflow")?
        )),
        sql: out.join("\n") + "\n",
        destructive,
        review,
    })
}
pub async fn run(allow: bool) -> Result<()> {
    let migrator = sqlx::migrate::Migrator::new(Path::new("migrations"))
        .await
        .map_err(|_| anyhow::anyhow!("invalid migrations"))?;
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL required")?;
    let pool = mach_db::connect(&url, 1).await?;
    if !allow
        && migrator
            .iter()
            .any(|m| m.sql.lines().any(|l| l == "-- mach-destructive"))
    {
        use sqlx::migrate::Migrate;
        let mut conn = pool.acquire().await.map_err(mach_db::Error::from)?;
        conn.ensure_migrations_table()
            .await
            .map_err(|_| anyhow::anyhow!("cannot read migration history"))?;
        let applied: BTreeSet<_> = conn
            .list_applied_migrations()
            .await
            .map_err(|_| anyhow::anyhow!("cannot read migration history"))?
            .into_iter()
            .map(|m| m.version)
            .collect();
        if migrator.iter().any(|m| {
            !applied.contains(&m.version) && m.sql.lines().any(|l| l == "-- mach-destructive")
        }) {
            bail!("pending destructive migration; review SQL and pass --allow-destructive");
        }
    }
    migrator
        .run(&pool)
        .await
        .context("migration failed; review the SQL and the migration history")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn managed_reference_follows_a_renamed_target() {
        let account = mach_codegen::entity(
            &syn::parse_quote!(
                struct Account {
                    #[id]
                    #[generated]
                    id: i64,
                }
            ),
            "accounts".into(),
        )
        .unwrap();
        let event = mach_codegen::entity(
            &syn::parse_quote!(
                struct Event {
                    #[id]
                    #[generated]
                    id: i64,
                    #[references("accounts.id")]
                    owner: Option<i64>,
                }
            ),
            "events".into(),
        )
        .unwrap();
        let old = Schema::from([("accounts".into(), account), ("events".into(), event)]);
        let mut new = old.clone();
        new.get_mut("accounts").unwrap().fields[0].name = "account_id".into();
        new.get_mut("events").unwrap().fields[1].references = Some("accounts.account_id".into());
        assert!(same_reference(
            &old,
            &new,
            &old["events"].fields[1],
            &new["events"].fields[1]
        ));
        new.get_mut("accounts").unwrap().fields[0].key = "replacement".into();
        assert!(!same_reference(
            &old,
            &new,
            &old["events"].fields[1],
            &new["events"].fields[1]
        ));
    }
}
