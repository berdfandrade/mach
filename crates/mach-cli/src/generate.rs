use crate::transaction::{Plan, format};
use anyhow::{Context, Result, ensure};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::{fs, path::Path, process::Command};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token, bracketed, punctuated::Punctuated};
macro_rules! template {
    ($name:literal) => {
        include_str!(concat!("../../../templates/", $name))
    };
}
pub fn new(path: &Path) -> Result<()> {
    ensure!(!path.exists(), "destination already exists");
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .context("invalid project name")?;
    ensure!(
        mach_codegen::valid_ident(&name.replace('-', "_")),
        "invalid project name"
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = tempfile::Builder::new()
        .prefix("mach-new-")
        .tempdir_in(parent)?;
    let root = temp.path();
    fs::create_dir_all(root.join("src/modules"))?;
    fs::create_dir_all(root.join("migrations"))?;
    fs::create_dir_all(root.join("tests"))?;
    let framework = std::env::var_os("MACH_FRAMEWORK_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../mach"))
        .canonicalize()
        .context("set MACH_FRAMEWORK_PATH to the public mach crate")?;
    let dependency =
        toml::Value::String(framework.to_str().context("invalid framework path")?.into());
    fs::write(
        root.join("Cargo.lock"),
        include_bytes!("../../../Cargo.lock"),
    )?;
    let manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nmach = {{ path = {dependency} }}\n\n[dev-dependencies]\ntower = {{ version = \"0.5\", features = [\"util\"] }}\n\n[workspace]\n"
    );
    for (p, s) in [
        ("Cargo.toml", manifest),
        ("mach.toml", template!("mach.toml").replace("PROJECT", name)),
        ("AGENTS.md", include_str!("../../../AGENTS.md").into()),
        ("CLAUDE.md", "@AGENTS.md\n".into()),
        (".gitignore", "/target/\n/.mach/\n.env\n".into()),
        (
            ".env.example",
            "DATABASE_URL=\nPOSTGRES_PASSWORD=\nMACH_TOKEN=\nMACH_GRANTS=\n".into(),
        ),
        ("compose.yaml", template!("compose.yaml").into()),
        ("src/main.rs", format(template!("main.rs"))?),
        ("src/app.rs", format(template!("app.rs"))?),
        (
            "src/modules/mod.rs",
            "mach::modules!(declared: [], routed: []);\n".into(),
        ),
    ] {
        fs::write(root.join(p), s)?
    }
    ensure!(
        Command::new("cargo")
            .args(["check", "--quiet", "--all-targets"])
            .current_dir(root)
            .status()?
            .success(),
        "generated app failed verification"
    );
    ensure!(!path.exists(), "destination appeared during generation");
    fs::rename(root, path)?;
    Ok(())
}
struct Group {
    names: Punctuated<Ident, Token![,]>,
    close: usize,
}
impl Group {
    fn parse(s: ParseStream, key: &str) -> syn::Result<Self> {
        let id: Ident = s.parse()?;
        if id != key {
            return Err(s.error("unexpected module registry"));
        }
        s.parse::<Token![:]>()?;
        let inner;
        let brackets = bracketed!(inner in s);
        Ok(Self {
            names: Punctuated::parse_terminated(&inner)?,
            close: brackets.span.close().byte_range().start,
        })
    }
    fn insert(&self, text: &mut String, module: &str) {
        if self.names.iter().any(|ident| ident == module) {
            return;
        }
        let separator = if self.names.is_empty() {
            ""
        } else if self.names.trailing_punct() {
            " "
        } else {
            ", "
        };
        text.insert_str(self.close, &format!("{separator}{module}"));
    }
}
struct Registry {
    declared: Group,
    routed: Group,
}
impl Parse for Registry {
    fn parse(s: ParseStream) -> syn::Result<Self> {
        let declared = Group::parse(s, "declared")?;
        s.parse::<Token![,]>()?;
        Ok(Self {
            declared,
            routed: Group::parse(s, "routed")?,
        })
    }
}
fn registry(file: &syn::File) -> Result<Registry> {
    file.items
        .iter()
        .find_map(|i| match i {
            syn::Item::Macro(m) if m.mac.path.segments.last()?.ident == "modules" => Some(&m.mac),
            _ => None,
        })
        .context("missing mach::modules! registry")?
        .parse_body()
        .map_err(Into::into)
}
pub fn routed_modules() -> Result<std::collections::BTreeSet<String>> {
    let text = fs::read_to_string("src/modules/mod.rs")?;
    Ok(registry(&syn::parse_file(&text)?)?
        .routed
        .names
        .iter()
        .map(ToString::to_string)
        .collect())
}
fn register(plan: &mut Plan, module: &str, routed: bool) -> Result<()> {
    let path = "src/modules/mod.rs";
    let text = fs::read_to_string(path)?;
    let groups = registry(&syn::parse_file(&text)?)?;
    let mut out = text.clone();
    if routed {
        groups.routed.insert(&mut out, module)
    }
    groups.declared.insert(&mut out, module);
    plan.stage_from(path, Some(text), out)
}
fn plural(word: &str) -> String {
    match word.strip_suffix('y') {
        Some(stem) if !stem.ends_with(['a', 'e', 'i', 'o', 'u']) => format!("{stem}ies"),
        _ if word.ends_with(['s', 'x', 'z']) || word.ends_with("ch") || word.ends_with("sh") => {
            format!("{word}es")
        }
        _ => format!("{word}s"),
    }
}
fn singular(word: &str) -> String {
    if let Some(stem) = word.strip_suffix("ies") {
        return format!("{stem}y");
    }
    if ["sses", "xes", "zes", "ches", "shes"]
        .iter()
        .any(|s| word.ends_with(s))
    {
        return word[..word.len() - 2].into();
    }
    match word.strip_suffix('s') {
        Some(stem) if !stem.ends_with('s') => stem.into(),
        _ => word.into(),
    }
}
pub fn component(kind: &str, name: &str, owner: Option<&str>) -> Result<Plan> {
    let (module, entity) = if kind == "entity" {
        ensure!(
            name == name.to_upper_camel_case(),
            "entity names use UpperCamelCase"
        );
        (plural(&name.to_snake_case()), name.to_owned())
    } else {
        (name.into(), singular(name).to_upper_camel_case())
    };
    ensure!(
        mach_codegen::valid_ident(&module) && syn::parse_str::<Ident>(&entity).is_ok(),
        "invalid module or entity name"
    );
    let dir = format!("src/modules/{module}");
    let mut plan = Plan::default();
    let mut mods = Vec::new();
    let crud = kind == "resource";
    let render = |s: &str| format(&s.replace("MODULE", &module).replace("ENTITY", &entity));
    let mut add = |unit: &str, source: String| -> Result<()> {
        let path = format!("{dir}/{unit}.rs");
        if !Path::new(&path).exists() {
            plan.create(&path, source)?
        }
        mods.push(unit.to_owned());
        Ok(())
    };
    if crud || kind == "entity" {
        let mut model = template!("model.rs").to_owned();
        if entity == "User" {
            model=model.replace("    #[writable]","    #[unique]\n    #[writable]\n    #[expose]\n    #[length(min = 3, max = 254)]\n    pub email: String,\n    #[writable]")
        }
        if let Some(table) = owner {
            ensure!(mach_codegen::valid_ident(table), "invalid owner table");
            model = model.replace(
                "    pub name: String,\n",
                &format!("    pub name: String,\n    #[owner]\n    #[expose]\n    #[references(\"{table}.id\")]\n    pub {}_id: i64,\n", singular(table)),
            );
        }
        add("model", render(&model)?)?;
    }
    if crud || kind == "controller" {
        let source = if crud && owner.is_some() {
            render(template!("controller_owned.rs"))?
        } else if crud {
            render(template!("controller.rs"))?
        } else {
            render(
                "use mach::prelude::*;\npub struct Controller;\n#[controller(\"/MODULE\")] impl Controller { #[get(\"\")] async fn status()->StatusCode{StatusCode::NO_CONTENT} }",
            )?
        };
        if crud && Path::new(&format!("{dir}/controller.rs")).exists() {
            let text = fs::read_to_string(format!("{dir}/controller.rs"))?;
            let actual = crate::index::parse(&text)?.routes;
            ensure!(
                crate::index::parse(&source)?
                    .routes
                    .iter()
                    .all(|route| actual.contains(route)),
                "controller exists; integrate CRUD handlers explicitly"
            );
        }
        add("controller", source)?;
    }
    if crud && entity == "User" {
        add("repository", render(template!("repository.rs"))?)?
    }
    if crud {
        add("tests", render(template!("tests.rs"))?)?
    }
    if kind == "service" {
        add("service", String::new())?
    }
    let path = format!("{dir}/mod.rs");
    let mut text = if Path::new(&path).exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let original = Path::new(&path).exists().then(|| text.clone());
    let file = syn::parse_file(&text)?;
    for m in mods {
        if file
            .items
            .iter()
            .any(|i| matches!(i,syn::Item::Mod(i) if i.ident==m))
        {
            continue;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n')
        }
        text.push_str(&if m == "tests" {
            "#[cfg(test)]\nmod tests;\n".into()
        } else {
            format!("pub mod {m};\n")
        });
    }
    plan.stage_from(&path, original, text)?;
    register(&mut plan, &module, crud || kind == "controller")?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn module_and_entity_names_round_trip() {
        for (module, entity) in [
            ("users", "user"),
            ("categories", "category"),
            ("addresses", "address"),
            ("boxes", "box"),
            ("matches", "match"),
            ("days", "day"),
        ] {
            assert_eq!(singular(module), entity);
            assert_eq!(plural(entity), module);
        }
        assert_eq!(singular("class"), "class");
    }
}
