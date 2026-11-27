use proc_macro2::Span;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use syn::{Attribute, Field as RustField, ItemStruct, LitBool, LitStr, Type, spanned::Spanned};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entity {
    pub name: String,
    pub table: String,
    pub fields: Vec<Field>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub key: String,
    pub kind: Kind,
    pub optional: bool,
    pub id: bool,
    #[serde(default)]
    pub generated: bool,
    #[serde(default)]
    pub owner: bool,
    pub write: bool,
    pub expose: bool,
    pub unique: bool,
    pub index: bool,
    pub default: Option<bool>,
    pub min: Option<usize>,
    pub max: Option<usize>,
    pub references: Option<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    String,
    I64,
    I32,
    Bool,
    Timestamp,
}
impl Kind {
    pub fn sql(self) -> &'static str {
        match self {
            Self::String => "TEXT",
            Self::I64 => "BIGINT",
            Self::I32 => "INTEGER",
            Self::Bool => "BOOLEAN",
            Self::Timestamp => "TIMESTAMPTZ",
        }
    }
    pub fn rust(self) -> &'static str {
        match self {
            Self::String => "String",
            Self::I64 => "i64",
            Self::I32 => "i32",
            Self::Bool => "bool",
            Self::Timestamp => "Timestamp",
        }
    }
}
pub fn valid_ident(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        && syn::parse_str::<syn::Ident>(s).is_ok()
}
pub fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn err<T>(span: Span, msg: &str) -> syn::Result<T> {
    Err(syn::Error::new(span, msg))
}
pub fn table_attr(attr: &Attribute) -> syn::Result<String> {
    let mut table = None;
    attr.parse_nested_meta(|m| {
        if !m.path.is_ident("table") || table.is_some() {
            return Err(m.error("expected table = \"name\""));
        }
        table = Some(m.value()?.parse::<LitStr>()?.value());
        Ok(())
    })?;
    table
        .filter(|s| valid_ident(s))
        .ok_or_else(|| syn::Error::new_spanned(attr, "expected a lowercase table identifier"))
}
pub fn entity(item: &ItemStruct, table: String) -> syn::Result<Entity> {
    if !valid_ident(&table) || !item.generics.params.is_empty() {
        return err(
            item.span(),
            "expected a concrete entity with a lowercase table name",
        );
    }
    let fields = item
        .fields
        .iter()
        .map(field)
        .collect::<syn::Result<Vec<_>>>()?;
    if fields.iter().filter(|f| f.id).count() != 1 {
        return err(item.span(), "declare one #[id] #[generated] i64 field");
    }
    if fields.iter().filter(|f| f.owner).count() > 1 {
        return err(item.span(), "declare at most one #[owner] field");
    }
    let mut keys = BTreeSet::new();
    if fields.iter().any(|f| !keys.insert(&f.key)) {
        return err(item.span(), "duplicate stable column key");
    }
    Ok(Entity {
        name: item.ident.to_string(),
        table,
        fields,
    })
}
fn field(f: &RustField) -> syn::Result<Field> {
    let name = f
        .ident
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    if !valid_ident(&name) {
        return err(f.span(), "expected a lowercase named field");
    }
    let mut ty = &f.ty;
    let mut optional = false;
    if let Type::Path(p) = ty
        && p.path.segments.len() == 1
        && p.path.segments[0].ident == "Option"
        && let syn::PathArguments::AngleBracketed(a) = &p.path.segments[0].arguments
        && a.args.len() == 1
        && let syn::GenericArgument::Type(inner) = &a.args[0]
    {
        optional = true;
        ty = inner
    }
    let kind = match ty {
        Type::Path(p) if p.path.is_ident("String") => Kind::String,
        Type::Path(p) if p.path.is_ident("i64") => Kind::I64,
        Type::Path(p) if p.path.is_ident("i32") => Kind::I32,
        Type::Path(p) if p.path.is_ident("bool") => Kind::Bool,
        Type::Path(p) if p.path.is_ident("Timestamp") => Kind::Timestamp,
        _ => {
            return err(
                ty.span(),
                "supported types: String, bool, i32, i64, Timestamp and Option<T>",
            );
        }
    };
    let mut out = Field {
        name: name.clone(),
        key: name,
        kind,
        optional,
        id: false,
        generated: false,
        owner: false,
        write: false,
        expose: false,
        unique: false,
        index: false,
        default: None,
        min: None,
        max: None,
        references: None,
    };
    let mut generated = false;
    let mut seen = BTreeSet::new();
    for a in &f.attrs {
        let attr = a
            .path()
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        if attr != "doc" && !seen.insert(attr.clone()) {
            return err(a.span(), "duplicate field attribute");
        }
        match attr.as_str() {
            "id" | "generated" | "owner" | "writable" | "expose" | "unique" | "index"
                if !matches!(a.meta, syn::Meta::Path(_)) =>
            {
                return err(a.span(), "unexpected arguments");
            }
            "id" => out.id = true,
            "generated" => generated = true,
            "owner" => out.owner = true,
            "writable" => out.write = true,
            "expose" => out.expose = true,
            "unique" => out.unique = true,
            "index" => out.index = true,
            "default" => out.default = Some(a.parse_args::<LitBool>()?.value),
            "column_key" => {
                out.key = a.parse_args::<LitStr>()?.value();
                if !valid_ident(&out.key) {
                    return err(a.span(), "invalid column key");
                }
            }
            "references" => {
                let v = a.parse_args::<LitStr>()?.value();
                if !v
                    .split_once('.')
                    .is_some_and(|(t, c)| valid_ident(t) && valid_ident(c))
                {
                    return err(a.span(), "expected table.column");
                }
                out.references = Some(v)
            }
            "length" => a.parse_nested_meta(|m| {
                let slot = if m.path.is_ident("min") {
                    &mut out.min
                } else if m.path.is_ident("max") {
                    &mut out.max
                } else {
                    return Err(m.error("expected min or max"));
                };
                if slot.is_some() {
                    return Err(m.error("duplicate bound"));
                }
                *slot = Some(m.value()?.parse::<syn::LitInt>()?.base10_parse()?);
                Ok(())
            })?,
            "doc" => {}
            _ => return err(a.span(), "unsupported entity field attribute"),
        }
    }
    if out.id && (!generated || out.optional || out.write || kind != Kind::I64) {
        return err(f.span(), "id must be generated, required, read-only i64");
    }
    out.generated = generated && !out.id;
    if out.generated && (kind != Kind::Timestamp || out.optional || out.write) {
        return err(
            f.span(),
            "#[generated] applies to the id or a required, read-only Timestamp",
        );
    }
    if out.owner && (out.id || out.optional || out.write || kind != Kind::I64) {
        return err(f.span(), "owner must be a required, read-only i64");
    }
    if out.default.is_some() && kind != Kind::Bool
        || (out.min.is_some() || out.max.is_some())
            && (kind != Kind::String || out.min.unwrap_or(0) > out.max.unwrap_or(usize::MAX))
    {
        return err(f.span(), "invalid default or length constraint");
    }
    if !(out.id || out.write || out.optional || out.generated || out.owner) && out.default.is_none()
    {
        return err(
            f.span(),
            "read-only fields require a default, Option<T>, #[generated] or #[owner]",
        );
    }
    Ok(out)
}
impl Field {
    pub fn column(&self) -> String {
        let c = quoted(&self.name);
        let mut sql = format!("{c} {}", self.kind.sql());
        if self.id {
            sql.push_str(" GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY")
        } else if !self.optional {
            sql.push_str(" NOT NULL")
        }
        if let Some(v) = self.default {
            sql.push_str(if v { " DEFAULT TRUE" } else { " DEFAULT FALSE" })
        }
        if self.generated {
            sql.push_str(" DEFAULT now()")
        }
        if let Some(r) = &self.references {
            let (t, f) = r.split_once('.').unwrap();
            sql.push_str(&format!(" REFERENCES {} ({})", quoted(t), quoted(f)))
        }
        sql
    }
}
#[derive(Debug, Serialize)]
pub struct Crud {
    pub find: String,
    pub create: String,
    pub update: Option<String>,
    pub list: String,
    pub list_desc: String,
    pub delete: String,
}
impl Entity {
    pub fn columns(&self) -> String {
        self.fields
            .iter()
            .map(|f| quoted(&f.name))
            .collect::<Vec<_>>()
            .join(", ")
    }
    pub fn crud(&self) -> Crud {
        let t = quoted(&self.table);
        let cols = self.columns();
        let id = quoted(&self.fields.iter().find(|f| f.id).unwrap().name);
        let writable = self.fields.iter().filter(|f| f.write).collect::<Vec<_>>();
        let owner = self.fields.iter().find(|f| f.owner);
        let inserted = writable.iter().copied().chain(owner).collect::<Vec<_>>();
        let names = inserted
            .iter()
            .map(|f| quoted(&f.name))
            .collect::<Vec<_>>()
            .join(", ");
        let bind = (1..=inserted.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let assignments = writable
            .iter()
            .enumerate()
            .map(|(i, f)| format!("{} = ${}", quoted(&f.name), i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let values = if inserted.is_empty() {
            "DEFAULT VALUES".into()
        } else {
            format!("({names}) VALUES ({bind})")
        };
        let scope = |n: usize| {
            let mut s = format!("{id} = ${n}");
            if let Some(o) = owner {
                s.push_str(&format!(" AND {} = ${}", quoted(&o.name), n + 1))
            }
            s
        };
        Crud {
            find: format!("SELECT {cols} FROM {t} WHERE {id} = $1"),
            create: format!("INSERT INTO {t} {values} RETURNING {cols}"),
            update: (!writable.is_empty()).then(|| {
                format!(
                    "UPDATE {t} SET {assignments} WHERE {} RETURNING {cols}",
                    scope(writable.len() + 1)
                )
            }),
            list: format!("SELECT {cols} FROM {t} ORDER BY {id} LIMIT $1 OFFSET $2"),
            list_desc: format!("SELECT {cols} FROM {t} ORDER BY {id} DESC LIMIT $1 OFFSET $2"),
            delete: format!("DELETE FROM {t} WHERE {}", scope(1)),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Find,
    Count,
    Exists,
    Delete,
}
pub struct Query<'a> {
    pub op: Op,
    pub sql: String,
    pub params: Vec<&'a Field>,
    pub fields: Vec<(&'a Field, &'static str)>,
}
pub fn derived<'a>(e: &'a Entity, name: &str) -> Result<Query<'a>, String> {
    let (op, tail) = [
        ("find_by_", Op::Find),
        ("count_by_", Op::Count),
        ("exists_by_", Op::Exists),
        ("delete_by_", Op::Delete),
    ]
    .into_iter()
    .find_map(|(prefix, op)| Some((op, name.strip_prefix(prefix)?)))
    .ok_or("unsupported query operation")?;
    let mut solutions = Vec::new();
    fn parse<'a>(
        fields: &'a [Field],
        tail: &str,
        path: &mut Vec<(&'a Field, &'static str)>,
        solutions: &mut Vec<Vec<(&'a Field, &'static str)>>,
    ) {
        if solutions.len() > 1 {
            return;
        }
        for f in fields {
            let Some(rest) = tail.strip_prefix(&f.name) else {
                continue;
            };
            for (suffix, op) in [
                ("_is_null", "IS NULL"),
                ("_greater_than", ">"),
                ("_less_than", "<"),
                ("", "="),
            ] {
                let Some(rest) = rest.strip_prefix(suffix) else {
                    continue;
                };
                path.push((f, op));
                if rest.is_empty() {
                    solutions.push(path.clone())
                } else if let Some(next) = rest.strip_prefix("_and_") {
                    parse(fields, next, path, solutions)
                }
                path.pop();
            }
        }
    }
    parse(&e.fields, tail, &mut Vec::new(), &mut solutions);
    if solutions.len() != 1 {
        return Err("unknown or ambiguous query field/operator".into());
    }
    let mut params = Vec::new();
    let mut conditions = Vec::new();
    let mut fields = Vec::new();
    for (f, cmp) in solutions.pop().unwrap() {
        fields.push((
            f,
            match cmp {
                "IS NULL" => "_is_null",
                ">" => "_greater_than",
                "<" => "_less_than",
                _ => "",
            },
        ));
        if cmp == "IS NULL" && !f.optional || matches!(cmp, "<" | ">") && f.kind == Kind::Bool {
            return Err("operator incompatible with field".into());
        }
        let mut c = format!("{} {cmp}", quoted(&f.name));
        if cmp != "IS NULL" {
            params.push(f);
            c.push_str(&format!(" ${}", params.len()))
        }
        conditions.push(c)
    }
    let from = format!(
        "FROM {} WHERE {}",
        quoted(&e.table),
        conditions.join(" AND ")
    );
    let id = quoted(&e.fields.iter().find(|f| f.id).unwrap().name);
    let sql = match op {
        Op::Find => format!("SELECT {} {from} ORDER BY {id} LIMIT 1", e.columns()),
        Op::Count => format!("SELECT COUNT(*) {from}"),
        Op::Exists => format!("SELECT EXISTS(SELECT 1 {from})"),
        Op::Delete => format!("DELETE {from}"),
    };
    Ok(Query {
        op,
        sql,
        params,
        fields,
    })
}

pub fn route_path(base: &str, suffix: &str) -> Result<String, &'static str> {
    if !base.starts_with('/')
        || base.ends_with('/')
        || !suffix.is_empty() && !suffix.starts_with('/')
    {
        return Err("use a base such as /users and a suffix such as /{id}");
    }
    let mut route = String::new();
    let mut params = BTreeSet::new();
    for segment in format!("{base}{suffix}").split('/').skip(1) {
        route.push('/');
        if let Some(name) = segment
            .strip_prefix(':')
            .or_else(|| segment.strip_prefix('{')?.strip_suffix('}'))
        {
            if !valid_ident(name) || !params.insert(name.to_owned()) {
                return Err("expected distinct lowercase path parameters");
            }
            route.push('{');
            route.push_str(name);
            route.push('}');
        } else if !segment.is_empty()
            && segment
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
        {
            route.push_str(segment)
        } else {
            return Err("expected a literal segment or {parameter}");
        }
    }
    Ok(route)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Entity {
        let src = syn::parse_quote!(
            struct Record {
                #[id]
                #[generated]
                id: i64,
                flag: Option<bool>,
                score: Option<i32>,
                alias: Option<String>,
            }
        );
        entity(&src, "records".into()).unwrap()
    }
    #[test]
    fn grammar_and_parameter_order() {
        let e = model();
        let q = derived(
            &e,
            "count_by_alias_is_null_and_score_greater_than_and_id_less_than",
        )
        .unwrap();
        assert_eq!(
            q.sql,
            "SELECT COUNT(*) FROM \"records\" WHERE \"alias\" IS NULL AND \"score\" > $1 AND \"id\" < $2"
        );
        assert_eq!(
            q.params.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["score", "id"]
        );
        for name in [
            "find_by_unknown",
            "delete_by_score_or_id",
            "count_by_id_is_null",
            "exists_by_flag_less_than",
        ] {
            assert!(derived(&e, name).is_err())
        }
    }
    #[test]
    fn owner_scopes_writes_and_timestamps_default_to_now() {
        let e = entity(
            &syn::parse_quote!(
                struct Note {
                    #[id]
                    #[generated]
                    id: i64,
                    #[writable]
                    body: String,
                    #[owner]
                    user_id: i64,
                    #[generated]
                    created_at: Timestamp,
                }
            ),
            "notes".into(),
        )
        .unwrap();
        let sql = e.crud();
        assert!(sql.create.starts_with(
            "INSERT INTO \"notes\" (\"body\", \"user_id\") VALUES ($1, $2) RETURNING"
        ));
        assert!(
            sql.update
                .unwrap()
                .contains("WHERE \"id\" = $2 AND \"user_id\" = $3")
        );
        assert_eq!(
            sql.delete,
            "DELETE FROM \"notes\" WHERE \"id\" = $1 AND \"user_id\" = $2"
        );
        assert!(sql.list_desc.contains("ORDER BY \"id\" DESC"));
        assert_eq!(
            e.fields[3].column(),
            "\"created_at\" TIMESTAMPTZ NOT NULL DEFAULT now()"
        );
        for invalid in [
            syn::parse_quote!(
                struct A {
                    #[id]
                    #[generated]
                    id: i64,
                    #[owner]
                    #[writable]
                    o: i64,
                }
            ),
            syn::parse_quote!(
                struct A {
                    #[id]
                    #[generated]
                    id: i64,
                    #[generated]
                    at: String,
                }
            ),
            syn::parse_quote!(
                struct A {
                    #[id]
                    #[generated]
                    id: i64,
                    #[owner]
                    a: i64,
                    #[owner]
                    b: i64,
                }
            ),
        ] {
            assert!(entity(&invalid, "a".into()).is_err())
        }
    }
    #[test]
    fn ambiguous_paths_are_rejected() {
        let mut e = model();
        let mut f = e.fields[0].clone();
        f.name = "score_and_id".into();
        e.fields.push(f);
        assert!(derived(&e, "find_by_score_and_id").is_err())
    }
}
