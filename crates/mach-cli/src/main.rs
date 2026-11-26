mod edit;
mod generate;
mod guard;
mod index;
mod migrations;
mod transaction;

use anyhow::{Result, bail, ensure};
use clap::{Parser, Subcommand};
use edit::{Edit, FieldCmd};
use index::Index;
use serde::Serialize;
use serde_json::{Value, json};
use std::{path::PathBuf, process::Command};

#[derive(Parser)]
#[command(version, about = "MACH application tooling")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    dry_run: bool,
    #[arg(long, global = true)]
    allow_destructive: bool,
    #[command(subcommand)]
    command: Cmd,
}
#[derive(Subcommand)]
enum Cmd {
    New {
        path: PathBuf,
        #[arg(long,default_value="postgres",value_parser=["postgres"])]
        db: String,
    },
    Generate {
        #[command(subcommand)]
        kind: Generate,
    },
    Field {
        #[command(subcommand)]
        op: FieldCmd,
    },
    Index {
        #[command(subcommand)]
        op: IndexCmd,
    },
    Context {
        #[arg(long)]
        compact: bool,
    },
    Inspect {
        entity: String,
        #[arg(long)]
        sql: bool,
    },
    Routes {
        module: Option<String>,
    },
    Dependencies,
    Schema,
    Api {
        symbol: String,
    },
    Check {
        #[arg(long)]
        agent: bool,
        #[arg(long)]
        strict: bool,
    },
    Fix {
        #[arg(long, required = true)]
        safe: bool,
    },
    Test {
        #[arg(long)]
        affected: bool,
    },
    Build {
        #[arg(long)]
        release: bool,
    },
    Dev,
    Expand,
    Migrate,
}
#[derive(Subcommand)]
enum IndexCmd {
    Add {
        entity: String,
        field: String,
        #[arg(long)]
        unique: bool,
    },
}
#[derive(Subcommand)]
enum Generate {
    Module {
        name: String,
    },
    Entity {
        name: String,
    },
    Controller {
        name: String,
    },
    Service {
        name: String,
    },
    Resource {
        name: String,
        #[arg(long, required = true)]
        crud: bool,
    },
    Migration,
}
fn print(v: impl Serialize, compact: bool) -> Result<()> {
    let v = serde_json::to_value(v)?;
    if compact {
        println!("{}", serde_json::to_string(&v)?)
    } else {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    println!(
                        "{k}: {}",
                        match v {
                            Value::String(s) => s,
                            _ => v.to_string(),
                        }
                    )
                }
            }
            _ => println!("{}", serde_json::to_string_pretty(&v)?),
        }
    }
    Ok(())
}
fn cargo(args: &[&str]) -> Result<()> {
    ensure!(
        Command::new("cargo").args(args).status()?.success(),
        "cargo {} failed",
        args[0]
    );
    Ok(())
}
fn affected() -> Result<()> {
    let diff = Command::new("git")
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()?;
    if diff.status.success() {
        let s = String::from_utf8(diff.stdout)?;
        let mut module = None;
        let mut only_tests = true;
        for line in s.split('\0').filter(|s| !s.is_empty()) {
            let Some((m, "tests.rs")) = line
                .get(3..)
                .and_then(|v| v.strip_prefix("src/modules/"))
                .and_then(|v| v.split_once('/'))
            else {
                only_tests = false;
                break;
            };
            if module.is_some_and(|v| v != m) {
                only_tests = false;
                break;
            }
            module = Some(m);
        }
        if only_tests && let Some(m) = module {
            return cargo(&["test", &format!("modules::{m}::tests::")]);
        }
    }
    cargo(&["test"])
}
fn api(symbol: &str) -> Result<Value> {
    let decl = match symbol {
        "Entity" => include_str!("../../mach-macros/src/lib.rs")
            .lines()
            .filter(|s| s.trim_start().starts_with("pub async fn"))
            .map(|s| {
                s.trim()
                    .trim_end_matches('{')
                    .replace("#input", "EntityInput")
            })
            .collect::<Vec<_>>(),
        "Controller" => vec![
            "#[controller(\"/resource\")] impl Controller",
            "#[get(\"/{id}\")] async fn find(State(s): State<AppState>, Path(id): Path<i64>)",
            "Controller::routes() -> Router<AppState>",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        "Repository" => vec![
            "#[repository(Entity)] pub trait Queries",
            "find_by -> Result<Option<Entity>>",
            "exists_by -> Result<bool>",
            "count_by -> Result<i64>",
            "delete_by -> Result<u64>",
            "operators: and, is_null, greater_than, less_than",
            "Queries::method(db, ...parameters)",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        _ => bail!("supported symbols: Entity, Repository, Controller"),
    };
    Ok(json!({"symbol":symbol,"declarations":decl}))
}
fn main() {
    if let Err(e) = run() {
        eprintln!("mach: {e:#}");
        std::process::exit(1)
    }
}
fn run() -> Result<()> {
    let cli = Cli::parse();
    let json = cli.json;
    let dry = cli.dry_run;
    let allow = cli.allow_destructive;
    if dry
        && !matches!(
            &cli.command,
            Cmd::Generate { .. } | Cmd::Field { .. } | Cmd::Index { .. } | Cmd::Fix { .. }
        )
    {
        bail!("--dry-run is supported for structural edits, generation and safe fixes")
    }
    if let Cmd::New { path, .. } = &cli.command {
        generate::new(path)?;
        return print(json!({"project":path}), json);
    }
    if let Cmd::Api { symbol } = &cli.command {
        return print(api(symbol)?, json);
    }
    if !std::path::Path::new("mach.toml").exists()
        && let Ok(manifest) = std::fs::read_to_string("Cargo.toml")
        && let Ok(manifest) = manifest.parse::<toml::Table>()
        && manifest.contains_key("workspace")
        && !manifest.contains_key("package")
    {
        return workspace(cli.command, json);
    }
    let config = mach_core::Config::load("mach.toml")?;
    let json = json || config.agent.output == mach_core::Output::Json;
    let _lock = transaction::lock()?;
    match cli.command {
        Cmd::Expand => cargo(&["expand"]),
        Cmd::Dev => {
            drop(_lock);
            cargo(&["run"])
        }
        Cmd::Build { release } => cargo(if release {
            &["build", "--release"]
        } else {
            &["build"]
        }),
        Cmd::Test { affected: true } => affected(),
        Cmd::Test { affected: false } => cargo(&["test"]),
        Cmd::Migrate => tokio::runtime::Runtime::new()?.block_on(migrations::run(allow)),
        Cmd::Generate { kind } => {
            let plan = match kind {
                Generate::Migration => {
                    let mut m = migrations::diff(&Index::load()?)?;
                    if dry {
                        return print(
                            json!({"file":m.path,"sql":m.sql,"destructive":m.destructive,"review":m.review}),
                            json,
                        );
                    }
                    let review = std::mem::take(&mut m.review);
                    let plan = m.plan(allow)?;
                    let files = json!(plan.paths());
                    plan.apply(true)?;
                    return print(json!({"files":files,"review":review}), json);
                }
                Generate::Resource { name, .. } => generate::component("resource", &name)?,
                Generate::Module { name } => generate::component("module", &name)?,
                Generate::Entity { name } => generate::component("entity", &name)?,
                Generate::Controller { name } => generate::component("controller", &name)?,
                Generate::Service { name } => generate::component("service", &name)?,
            };
            let files = json!(plan.paths());
            if !dry {
                plan.apply(true)?
            }
            print(json!({"files":files}), json)
        }
        Cmd::Field { op } => edit(&Index::load()?, Edit::Field(op), dry, allow, json),
        Cmd::Index {
            op:
                IndexCmd::Add {
                    entity,
                    field,
                    unique,
                },
        } => edit(
            &Index::load()?,
            Edit::Index {
                entity,
                field,
                unique,
            },
            dry,
            allow,
            json,
        ),
        Cmd::Fix { .. } => {
            let plan = guard::fix()?;
            let files = json!(plan.paths());
            if !dry {
                plan.apply(true)?
            }
            print(json!({"files":files}), json)
        }
        Cmd::Check { agent, strict } => {
            let findings = guard::scan(&config, strict)?;
            print(json!({"diagnostics":findings}), json || agent)?;
            guard::verify(&findings)
        }
        cmd => {
            let idx = Index::load()?;
            let val = match cmd {
                Cmd::Context { compact } => {
                    let summary = json!({"mach":env!("CARGO_PKG_VERSION"),"db":config.database.provider,"modules":idx.modules(),"entities":idx.entities().map(|(_,e)|&e.name).collect::<Vec<_>>(),"routes":idx.routes(None)?.len(),"schema_changes":migrations::diff(&idx)?.path.is_some()});
                    if !compact && config.agent.output == mach_core::Output::Human && !json {
                        println!("{}", serde_json::to_string_pretty(&summary)?);
                        return Ok(());
                    }
                    summary
                }
                Cmd::Inspect { entity, sql } => {
                    let (p, e) = idx.find(&entity)?;
                    let queries = idx.queries(e)?;
                    if !json {
                        println!("{} ({})", e.name, e.table);
                        for f in &e.fields {
                            let mut flags = Vec::new();
                            if f.id {
                                flags.push("PK AUTO".to_owned())
                            }
                            if f.unique {
                                flags.push("UNIQUE".to_owned())
                            } else if f.index {
                                flags.push("INDEX".to_owned())
                            }
                            if let Some(max) = f.max {
                                flags.push(format!("MAX({max})"))
                            }
                            if let Some(min) = f.min {
                                flags.push(format!("MIN({min})"))
                            }
                            if let Some(d) = f.default {
                                flags.push(format!("DEFAULT({d})"))
                            }
                            if f.write {
                                flags.push("WRITE".to_owned())
                            }
                            if f.expose {
                                flags.push("EXPOSE".to_owned())
                            }
                            if let Some(r) = &f.references {
                                flags.push(format!("REF({r})"))
                            }
                            println!(
                                "  {}:{}{} {}",
                                f.name,
                                f.kind.rust(),
                                if f.optional { "?" } else { "" },
                                flags.join(" ")
                            );
                        }
                        if sql {
                            print(e.crud(), false)?;
                            print(&queries, false)?;
                        } else {
                            println!(
                                "queries: {}",
                                queries.keys().cloned().collect::<Vec<_>>().join(",")
                            );
                        }
                        return Ok(());
                    }
                    if sql {
                        json!({"file":p,"entity":e,"crud":e.crud(),"queries":queries})
                    } else {
                        json!({"file":p,"entity":e,"queries":queries.keys().collect::<Vec<_>>()})
                    }
                }
                Cmd::Routes { module } => json!(idx.routes(module.as_deref())?),
                Cmd::Dependencies => json!(
                    idx.files
                        .iter()
                        .map(|(p, v)| (p, &v.dependencies))
                        .collect::<std::collections::BTreeMap<_, _>>()
                ),
                Cmd::Schema => json!(idx.schema()),
                _ => unreachable!(),
            };
            print(val, json)
        }
    }
}
fn edit(idx: &Index, op: Edit, dry: bool, allow: bool, json: bool) -> Result<()> {
    let (plan, impact) = edit::change(idx, op, dry, allow)?;
    if !dry {
        plan.apply(true)?
    }
    print(impact, json)
}

fn workspace(cmd: Cmd, json: bool) -> Result<()> {
    match cmd {
        Cmd::Context { .. } => {
            let output = Command::new("cargo")
                .args(["metadata", "--no-deps", "--format-version=1"])
                .output()?;
            ensure!(output.status.success(), "cargo metadata failed");
            let metadata: Value = serde_json::from_slice(&output.stdout)?;
            let packages = metadata["packages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p["name"].as_str())
                .collect::<std::collections::BTreeSet<_>>();
            print(
                json!({"mach":env!("CARGO_PKG_VERSION"),"packages":packages}),
                json,
            )
        }
        Cmd::Test { .. } => cargo(&["test", "--workspace"]),
        Cmd::Build { release } => cargo(if release {
            &["build", "--workspace", "--release"]
        } else {
            &["build", "--workspace"]
        }),
        Cmd::Check { agent, strict } => {
            let mut findings = guard::cargo_checks(&mach_core::Policy::default())?;
            if strict {
                for file in walkdir::WalkDir::new(".").into_iter().filter_entry(|e| {
                    e.file_name() != "target" && e.file_name() != ".git" && e.file_name() != ".mach"
                }) {
                    let file = file?;
                    if file.file_type().is_file()
                        && file.path().extension().is_some_and(|e| e == "rs")
                    {
                        let path = file.path();
                        findings.extend(mach_guard::analyze(
                            &path.to_string_lossy(),
                            &std::fs::read_to_string(path)?,
                            true,
                        )?);
                    }
                }
                findings.sort();
            }
            print(json!({"diagnostics":findings}), json || agent)?;
            guard::verify(&findings)
        }
        _ => bail!("run application commands in the directory containing mach.toml"),
    }
}
