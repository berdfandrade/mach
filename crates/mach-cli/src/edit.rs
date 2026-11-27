use crate::{index::Index, transaction::Plan};
use anyhow::{Context, Result, bail, ensure};
use mach_codegen::{Kind, derived, valid_ident};
use proc_macro2::Span;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    ops::Range,
};
use syn::{Item, spanned::Spanned, visit::Visit};
#[derive(clap::Subcommand)]
pub enum FieldCmd {
    Add {
        entity: String,
        spec: String,
        #[arg(long)]
        optional: bool,
        #[arg(long)]
        writable: bool,
        #[arg(long)]
        expose: bool,
        #[arg(long)]
        generated: bool,
    },
    Rename {
        entity: String,
        field: String,
        new_name: String,
    },
    Alter {
        entity: String,
        field: String,
        #[arg(long)]
        max_length: usize,
    },
    Remove {
        entity: String,
        field: String,
    },
}
pub enum Edit {
    Field(FieldCmd),
    Index {
        entity: String,
        field: String,
        unique: bool,
    },
}
#[derive(Default, Serialize)]
pub struct Impact {
    entity: String,
    files: Vec<String>,
    managed_references: Vec<String>,
    queries: BTreeSet<String>,
    crud: Vec<String>,
    migration: String,
    api_contract: bool,
    destructive: bool,
    unresolved: BTreeSet<String>,
    analysis: &'static str,
}
type Edits = BTreeMap<String, Vec<(Range<usize>, String)>>;
fn replace(edits: &mut Edits, p: &str, range: Range<usize>, text: impl Into<String>) {
    edits
        .entry(p.into())
        .or_default()
        .push((range, text.into()))
}
fn insert_attr(edits: &mut Edits, p: &str, src: &str, span: Span, attr: &str) {
    let pos = span.byte_range().start;
    let line = &src[..pos].rsplit_once('\n').map_or(&src[..pos], |(_, v)| v);
    let indent = if line.chars().all(char::is_whitespace) {
        line
    } else {
        " "
    };
    replace(edits, p, pos..pos, format!("#[{attr}]\n{indent}"));
}
struct References<'a> {
    name: &'a str,
    found: Vec<Span>,
}
impl<'ast> Visit<'ast> for References<'_> {
    fn visit_ident(&mut self, id: &'ast syn::Ident) {
        if id == self.name {
            self.found.push(id.span())
        }
    }
}
pub fn change(index: &Index, op: Edit, dry: bool, allow: bool) -> Result<(Plan, Impact)> {
    use FieldCmd::*;
    let (name, key) = match &op {
        Edit::Field(Add { entity, spec, .. }) => (
            entity,
            spec.split_once(':').context("expected name:type")?.0,
        ),
        Edit::Field(
            Rename { entity, field, .. } | Alter { entity, field, .. } | Remove { entity, field },
        )
        | Edit::Index { entity, field, .. } => (entity, field.as_str()),
    };
    ensure!(valid_ident(key), "invalid field name");
    let (path, model) = index.find(name)?;
    let src = fs::read_to_string(path)?;
    let file = syn::parse_file(&src)?;
    let item = file
        .items
        .iter()
        .find_map(|i| match i {
            Item::Struct(st) if st.ident == name => Some(st),
            _ => None,
        })
        .context("semantic edits require a top-level entity")?;
    let syn::Fields::Named(fields) = &item.fields else {
        bail!("expected named fields")
    };
    let existing = fields
        .named
        .iter()
        .find(|f| f.ident.as_ref().is_some_and(|i| i == key));
    let meta = model.fields.iter().find(|f| f.name == key);
    let mut originals = BTreeMap::from([(path.to_owned(), src.clone())]);
    let mut edits = Edits::new();
    let mut impact = Impact {
        entity: name.clone(),
        api_contract: meta.is_some_and(|f| f.write || f.expose),
        analysis: "managed declarations; arbitrary Rust and SQL require review",
        ..Default::default()
    };
    match &op {
        Edit::Field(Add {
            spec,
            optional,
            writable,
            expose,
            generated,
            ..
        }) => {
            ensure!(existing.is_none(), "field already exists");
            let (_, kind) = spec.split_once(':').unwrap();
            let ty = match kind {
                "string" => "String",
                "bool" => "bool",
                "i32" => "i32",
                "i64" => "i64",
                "timestamp" => "Timestamp",
                _ => bail!("unsupported field type"),
            };
            ensure!(
                !*generated || ty == "Timestamp" && !*optional && !*writable,
                "--generated applies to a required, read-only timestamp"
            );
            ensure!(
                *optional || *writable || *generated,
                "a required field needs an explicit write policy; use --writable, --optional or --generated"
            );
            let ty = if *optional {
                format!("Option<{ty}>")
            } else {
                ty.into()
            };
            let mut text = String::new();
            if *generated {
                text.push_str("    #[generated]\n")
            }
            if *writable {
                text.push_str("    #[writable]\n")
            }
            if *expose {
                text.push_str("    #[expose]\n")
            }
            text.push_str(&format!("    pub {key}: {ty},\n"));
            let pos = fields.brace_token.span.close().byte_range().start;
            if !src[..pos].ends_with('\n') {
                text.insert(0, '\n')
            }
            if !fields.named.is_empty() && !fields.named.trailing_punct() {
                let end = fields.named.last().unwrap().span().byte_range().end;
                replace(&mut edits, path, end..end, ",")
            }
            replace(&mut edits, path, pos..pos, text);
            impact.api_contract = *writable || *expose;
            impact.migration = "add column".into();
        }
        _ => {
            let field = existing.context("unknown field")?;
            let f = meta.unwrap();
            let id = field.ident.as_ref().unwrap();
            match &op {
                Edit::Field(Rename { new_name, .. }) => {
                    ensure!(
                        valid_ident(new_name) && !model.fields.iter().any(|f| f.name == *new_name),
                        "invalid or duplicate destination field"
                    );
                    if !field.attrs.iter().any(|a| a.path().is_ident("column_key")) {
                        insert_attr(
                            &mut edits,
                            path,
                            &src,
                            field.span(),
                            &format!("column_key(\"{key}\")"),
                        )
                    }
                    replace(&mut edits, path, id.span().byte_range(), new_name);
                    impact.migration = "rename column".into();
                }
                Edit::Field(Remove { .. }) => {
                    ensure!(!f.id, "cannot remove the primary key");
                    impact.destructive = true;
                    ensure!(
                        dry || allow,
                        "removing a field requires --allow-destructive"
                    );
                    let mut range = field.span().byte_range();
                    if let Some(comma) = fields
                        .named
                        .pairs()
                        .find(|pair| pair.value().ident == field.ident)
                        .and_then(|pair| pair.punct().copied())
                    {
                        range.end = comma.span().byte_range().end
                    }
                    ensure!(
                        !src[range.clone()].contains("//") && !src[range.clone()].contains("/*"),
                        "field contains comments; review and move them before removal"
                    );
                    if src[..range.start]
                        .rsplit('\n')
                        .next()
                        .unwrap()
                        .trim()
                        .is_empty()
                    {
                        range.start = src[..range.start].rfind('\n').map_or(0, |i| i + 1)
                    }
                    if src[range.end..].starts_with('\n') {
                        range.end += 1
                    }
                    replace(&mut edits, path, range, "");
                    impact.migration = "drop column".into();
                }
                Edit::Field(Alter { max_length, .. }) => {
                    ensure!(
                        f.kind == Kind::String && f.min.unwrap_or(0) <= *max_length,
                        "invalid length bound"
                    );
                    if let Some(a) = field.attrs.iter().find(|a| a.path().is_ident("length")) {
                        let mut changed = false;
                        a.parse_nested_meta(|m| {
                            let n = m.value()?.parse::<syn::LitInt>()?;
                            if m.path.is_ident("max") {
                                replace(
                                    &mut edits,
                                    path,
                                    n.span().byte_range(),
                                    max_length.to_string(),
                                );
                                changed = true
                            }
                            Ok(())
                        })?;
                        if !changed {
                            let list = a.meta.require_list()?;
                            let end = list.delimiter.span().close().byte_range().start;
                            replace(&mut edits, path, end..end, format!(", max = {max_length}"))
                        }
                    } else {
                        insert_attr(
                            &mut edits,
                            path,
                            &src,
                            field.span(),
                            &format!("length(max = {max_length})"),
                        )
                    }
                    impact.migration = "change length constraint".into();
                }
                Edit::Index { unique, .. } => {
                    if !(f.unique || f.index && !unique) {
                        insert_attr(
                            &mut edits,
                            path,
                            &src,
                            field.span(),
                            if *unique { "unique" } else { "index" },
                        )
                    }
                    impact.api_contract = false;
                    impact.migration = "create index".into();
                }
                _ => unreachable!(),
            }
        }
    }
    if matches!(&op, Edit::Field(Rename { .. } | Remove { .. })) {
        for p in index.files.keys() {
            let text = fs::read_to_string(p)?;
            originals.entry(p.clone()).or_insert_with(|| text.clone());
            let file = syn::parse_file(&text)?;
            for i in &file.items {
                if let Item::Trait(t) = i
                    && t.attrs.iter().any(|a| {
                        a.path().is_ident("repository")
                            && a.parse_args::<syn::Path>()
                                .ok()
                                .and_then(|p| p.segments.last().map(|s| s.ident.to_string()))
                                .as_ref()
                                == Some(name)
                    })
                {
                    for m in &t.items {
                        if let syn::TraitItem::Fn(m) = m {
                            let old = m.sig.ident.to_string();
                            let query = derived(model, &old).map_err(anyhow::Error::msg)?;
                            if query.fields.iter().any(|(f, _)| f.name == key) {
                                impact.queries.insert(old.clone());
                                if let Edit::Field(Rename { new_name, .. }) = &op {
                                    let prefix = old.split_once("_by_").unwrap().0;
                                    let tail = query
                                        .fields
                                        .iter()
                                        .map(|(f, suffix)| {
                                            format!(
                                                "{}{}",
                                                if f.name == key { new_name } else { &f.name },
                                                suffix
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                        .join("_and_");
                                    replace(
                                        &mut edits,
                                        p,
                                        m.sig.ident.span().byte_range(),
                                        format!("{prefix}_by_{tail}"),
                                    );
                                } else if !dry {
                                    bail!("query {old} depends on removed field")
                                }
                            }
                        }
                    }
                }
                if let Item::Struct(st) = i
                    && st.attrs.iter().any(|a| a.path().is_ident("entity"))
                {
                    for field in &st.fields {
                        for a in &field.attrs {
                            if a.path().is_ident("references") {
                                let target = a.parse_args::<syn::LitStr>()?;
                                if target.value() == format!("{}.{key}", model.table) {
                                    impact.managed_references.push(format!(
                                        "{}.{}",
                                        st.ident,
                                        field.ident.as_ref().unwrap()
                                    ));
                                    if let Edit::Field(Rename { new_name, .. }) = &op {
                                        replace(
                                            &mut edits,
                                            p,
                                            target.span().byte_range(),
                                            format!("\"{}.{new_name}\"", model.table),
                                        )
                                    } else if !dry {
                                        bail!("referencing column requires review")
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let mut refs = References {
                name: key,
                found: vec![],
            };
            refs.visit_file(&file);
            for span in refs.found {
                if !(p == path
                    && existing.is_some_and(|f| {
                        f.ident.as_ref().unwrap().span().byte_range() == span.byte_range()
                    }))
                {
                    impact
                        .unresolved
                        .insert(format!("{p}:{}", span.start().line));
                }
            }
        }
    }
    let mut plan = Plan::default();
    for (p, mut changes) in edits {
        let original = originals.remove(&p).context("missing source snapshot")?;
        let mut text = original.clone();
        changes.sort_by_key(|(r, _)| (r.start, r.end));
        for (range, replacement) in changes.into_iter().rev() {
            text.replace_range(range, &replacement)
        }
        let file = syn::parse_file(&text)?;
        if p == path {
            let next = file
                .items
                .iter()
                .find_map(|i| match i {
                    Item::Struct(st) if st.ident == name => Some(st),
                    _ => None,
                })
                .context("entity removed")?;
            let next = mach_codegen::entity(next, model.table.clone())?;
            let before = serde_json::to_value(model.crud())?;
            let after = serde_json::to_value(next.crud())?;
            impact.crud = before
                .as_object()
                .unwrap()
                .iter()
                .filter(|(k, v)| after[*k] != **v)
                .map(|(k, _)| k.clone())
                .collect();
        }
        plan.stage_from(&p, Some(original), text)?;
    }
    impact.files = plan.paths().into_iter().map(String::from).collect();
    Ok((plan, impact))
}
