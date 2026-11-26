use anyhow::{Context, Result, ensure};
use mach_codegen::Entity;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use syn::{Attribute, Item, LitStr};
use walkdir::WalkDir;

#[derive(Default, Serialize, Deserialize, Clone)]
pub struct Entry {
    pub entities: Vec<Entity>,
    pub routes: Vec<(String, String)>,
    pub queries: Vec<(String, String)>,
    pub dependencies: BTreeSet<String>,
    pub policies: Vec<String>,
    pub tests: Vec<String>,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Index {
    pub files: BTreeMap<String, Entry>,
}
fn attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attrs
        .iter()
        .find(|a| a.path().segments.last().is_some_and(|s| s.ident == name))
}
pub(crate) fn parse(src: &str) -> Result<Entry> {
    let file = syn::parse_file(src)?;
    let mut entry = Entry::default();
    fn items(list: &[Item], e: &mut Entry) -> Result<()> {
        for item in list {
            match item {
                Item::Struct(st) if attr(&st.attrs, "entity").is_some() => {
                    let table = mach_codegen::table_attr(attr(&st.attrs, "entity").unwrap())?;
                    e.entities.push(mach_codegen::entity(st, table)?)
                }
                Item::Impl(i) if attr(&i.attrs, "controller").is_some() => {
                    let base = attr(&i.attrs, "controller")
                        .unwrap()
                        .parse_args::<LitStr>()?
                        .value();
                    for m in &i.items {
                        if let syn::ImplItem::Fn(m) = m {
                            for method in ["get", "post", "put", "patch", "delete"] {
                                if let Some(a) = attr(&m.attrs, method) {
                                    e.routes.push((
                                        method.to_uppercase(),
                                        mach_codegen::route_path(
                                            &base,
                                            &a.parse_args::<LitStr>()?.value(),
                                        )
                                        .map_err(anyhow::Error::msg)?,
                                    ))
                                }
                            }
                        }
                    }
                }
                Item::Impl(i)
                    if i.trait_.as_ref().is_some_and(|(_, p, _)| {
                        p.segments.last().is_some_and(|s| s.ident == "Access")
                    }) =>
                {
                    use quote::ToTokens;
                    e.policies.push(i.self_ty.to_token_stream().to_string())
                }
                Item::Trait(t) if attr(&t.attrs, "repository").is_some() => {
                    let p = attr(&t.attrs, "repository")
                        .unwrap()
                        .parse_args::<syn::Path>()?;
                    let n = p.segments.last().unwrap().ident.to_string();
                    for m in &t.items {
                        if let syn::TraitItem::Fn(f) = m {
                            e.queries.push((n.clone(), f.sig.ident.to_string()))
                        }
                    }
                }
                Item::Use(u) => {
                    use quote::ToTokens;
                    e.dependencies.insert(u.tree.to_token_stream().to_string());
                }
                Item::Fn(f) if attr(&f.attrs, "test").is_some() => {
                    e.tests.push(f.sig.ident.to_string())
                }
                Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        items(inner, e)?
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    items(&file.items, &mut entry)?;
    Ok(entry)
}
const CACHE: &str = ".mach/index-v2";
pub fn sources() -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for dir in ["src", "tests"] {
        if !Path::new(dir).exists() {
            continue;
        }
        for e in WalkDir::new(dir) {
            let e = e?;
            if e.path().extension().is_some_and(|s| s == "rs") {
                crate::transaction::checked(e.path())?;
                paths.push(e.path().to_str().context("invalid path")?.into())
            }
        }
    }
    paths.sort();
    Ok(paths)
}
impl Index {
    pub fn load() -> Result<Self> {
        let mut out = Self::default();
        let mut names = BTreeSet::new();
        let mut tables = BTreeSet::new();
        let mut live = BTreeSet::new();
        for p in sources()? {
            let text = fs::read_to_string(&p)?;
            let cache = format!("{CACHE}/{:x}.json", Sha256::digest(text.as_bytes()));
            crate::transaction::checked(Path::new(&cache))?;
            let entry: Entry = match fs::read(&cache)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
            {
                Some(e) => e,
                None => {
                    let e = parse(&text).with_context(|| format!("invalid source {p}"))?;
                    crate::transaction::atomic(Path::new(&cache), &serde_json::to_vec(&e)?)?;
                    e
                }
            };
            for e in &entry.entities {
                ensure!(
                    names.insert(e.name.clone()) && tables.insert(e.table.clone()),
                    "duplicate entity/table {}",
                    e.name
                )
            }
            out.files.insert(p, entry);
            live.insert(cache);
        }
        for stale in fs::read_dir(CACHE).into_iter().flatten() {
            let path = stale?.path();
            if !live.iter().any(|l| Path::new(l) == path) {
                fs::remove_file(path)?
            }
        }
        Ok(out)
    }
    pub fn entities(&self) -> impl Iterator<Item = (&str, &Entity)> {
        self.files
            .iter()
            .flat_map(|(p, e)| e.entities.iter().map(move |v| (p.as_str(), v)))
    }
    pub fn find(&self, n: &str) -> Result<(&str, &Entity)> {
        self.entities()
            .find(|(_, e)| e.name == n)
            .context("unknown entity")
    }
    pub fn schema(&self) -> BTreeMap<String, Entity> {
        self.entities()
            .map(|(_, e)| (e.table.clone(), e.clone()))
            .collect()
    }
    pub fn routes(&self, module: Option<&str>) -> Result<Vec<(String, String)>> {
        let mounted = crate::generate::routed_modules()?;
        let mut routes = Vec::new();
        for (path, entry) in &self.files {
            if let Some((m, "controller.rs")) = path
                .strip_prefix("src/modules/")
                .and_then(|p| p.split_once('/'))
                && mounted.contains(m)
                && module.is_none_or(|wanted| wanted == m)
            {
                routes.extend(entry.routes.iter().cloned());
            }
        }
        if module.is_none() {
            routes.extend([
                ("GET".into(), "/health".into()),
                ("GET".into(), "/ready".into()),
            ])
        }
        routes.sort();
        Ok(routes)
    }
    pub fn modules(&self) -> BTreeSet<&str> {
        self.files
            .keys()
            .filter_map(|p| {
                p.strip_prefix("src/modules/")?
                    .split_once('/')
                    .map(|(m, _)| m)
            })
            .collect()
    }
    pub fn queries(&self, e: &Entity) -> Result<BTreeMap<String, String>> {
        self.files
            .values()
            .flat_map(|v| &v.queries)
            .filter(|(n, _)| n == &e.name)
            .map(|(_, q)| {
                Ok((
                    q.clone(),
                    mach_codegen::derived(e, q).map_err(anyhow::Error::msg)?.sql,
                ))
            })
            .collect()
    }
}
