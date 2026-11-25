use crate::{
    index,
    transaction::{Plan, format},
};
use anyhow::{Result, ensure};
use mach_core::{Config, Level};
use mach_guard::Finding;
use serde_json::Value;
use std::{fs, process::Command};
pub fn scan(config: &Config, strict: bool) -> Result<Vec<Finding>> {
    let mut out = architecture().unwrap_or_else(|_| {
        vec![Finding::new(
            "src",
            0,
            "MACH001",
            "proven",
            "invalid application declarations or metadata",
        )]
    });
    for p in index::sources()? {
        let analysis = mach_guard::analyze(
            &p,
            &fs::read_to_string(&p)?,
            strict || config.policy.redundant_comments == Level::Deny,
        );
        let findings = match analysis {
            Ok(v) => v,
            Err(e) => {
                out.push(Finding::new(
                    &p,
                    e.span().start().line,
                    "RUST",
                    "proven",
                    "invalid Rust syntax",
                ));
                continue;
            }
        };
        for mut f in findings {
            let level = match f.code.as_str() {
                "STYLE001" if strict => Level::Deny,
                "STYLE001" => config.policy.redundant_comments,
                "ARCH001" => config.policy.unnecessary_wrappers,
                "SEC001" => config.policy.unsafe_code,
                "ARCH004" => config.policy.speculative_abstractions,
                "PERF003" => config.policy.unbounded_public_queries,
                _ => Level::Warn,
            };
            if level == Level::Allow {
                continue;
            }
            f.severity = if level == Level::Deny && f.certainty == "proven" {
                "error"
            } else {
                "warning"
            };
            out.push(f)
        }
    }
    out.extend(cargo_checks(&config.policy)?);
    if config.policy.unused_dependencies != Level::Allow {
        let result = Command::new("cargo")
            .args([
                "rustc",
                "--bin",
                &config.project.name,
                "--message-format=json",
                "--",
                "-W",
                "unused_crate_dependencies",
            ])
            .output()?;
        for line in result.stdout.split(|b| *b == b'\n') {
            if let Ok(m) = serde_json::from_slice::<Value>(line)
                && m["message"]["code"]["code"] == "unused_crate_dependencies"
                && m["target"]["src_path"].as_str().is_some_and(|p| {
                    std::path::Path::new(p).starts_with(std::env::current_dir().unwrap_or_default())
                })
            {
                out.push(Finding::new(
                    "Cargo.toml",
                    0,
                    "ARCH005",
                    "advisory",
                    "a declared dependency is unused by this application binary",
                ));
            }
        }
    }
    out.retain(|f| {
        !(f.code.starts_with("ARCH")
            || f.code.starts_with("STYLE")
            || f.code.starts_with("PERF")
            || f.code.starts_with("SEC"))
            || !config
                .policy
                .exceptions
                .iter()
                .any(|e| e.code == f.code && e.file == f.file)
    });
    out.sort();
    out.dedup();
    Ok(out)
}
pub fn cargo_checks(policy: &mach_core::Policy) -> Result<Vec<Finding>> {
    let mut out = Vec::new();
    if !Command::new("cargo")
        .args(["fmt", "--all", "--", "--check"])
        .output()?
        .status
        .success()
    {
        out.push(Finding::new(
            "src",
            0,
            "FMT001",
            "proven",
            "cargo fmt --check failed",
        ))
    }
    let clippy = Command::new("cargo")
        .args([
            "clippy",
            "--all-targets",
            "--message-format=json",
            "--",
            "-W",
            "dead_code",
            "-W",
            "unused_imports",
            "-W",
            "clippy::redundant_clone",
        ])
        .output()?;
    for line in clippy.stdout.split(|b| *b == b'\n') {
        let Ok(msg) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if msg["reason"] != "compiler-message" {
            continue;
        }
        let m = &msg["message"];
        if !matches!(m["level"].as_str(), Some("warning" | "error")) {
            continue;
        }
        let lint = m["code"]["code"].as_str().unwrap_or("rustc");
        let (code, policy) = match lint {
            "dead_code" => ("STYLE003", policy.dead_code),
            "unused_imports" => ("STYLE002", Level::Warn),
            "clippy::redundant_clone" => ("PERF001", policy.unnecessary_clones),
            "unused_crate_dependencies" => ("ARCH005", policy.unused_dependencies),
            other => (other, Level::Warn),
        };
        if policy == Level::Allow && m["level"] != "error" {
            continue;
        }
        let s = &m["spans"]
            .as_array()
            .and_then(|s| s.iter().find(|s| s["is_primary"] == true))
            .cloned()
            .unwrap_or_default();
        let mut f = Finding::new(
            s["file_name"].as_str().unwrap_or("Cargo.toml"),
            s["line_start"].as_u64().unwrap_or(0) as usize,
            code,
            "proven",
            lint,
        );
        f.severity = if policy == Level::Deny || m["level"] == "error" {
            "error"
        } else {
            "warning"
        };
        out.push(f)
    }
    if !clippy.status.success() && !out.iter().any(|f| f.severity == "error") {
        out.push(Finding::new(
            "Cargo.toml",
            0,
            "CARGO",
            "proven",
            "cargo clippy failed",
        ))
    }
    out.sort();
    out.dedup();
    Ok(out)
}
pub fn fix() -> Result<Plan> {
    let mut plan = Plan::default();
    for p in index::sources()? {
        let s = fs::read_to_string(&p)?;
        let formatted = format(&s)?;
        plan.stage_from(&p, Some(s), formatted)?
    }
    Ok(plan)
}
pub fn verify(out: &[Finding]) -> Result<()> {
    ensure!(!out.iter().any(|f| f.severity == "error"), "check failed");
    Ok(())
}

fn module_imports(tree: &syn::UseTree, prefix: &str, out: &mut Vec<String>) {
    match tree {
        syn::UseTree::Path(p) => module_imports(&p.tree, &format!("{prefix}{}::", p.ident), out),
        syn::UseTree::Group(g) => {
            for t in &g.items {
                module_imports(t, prefix, out)
            }
        }
        syn::UseTree::Name(n) => out.push(format!("{prefix}{}", n.ident)),
        syn::UseTree::Rename(n) => out.push(format!("{prefix}{}", n.ident)),
        syn::UseTree::Glob(_) => out.push(format!("{prefix}*")),
    }
}
fn architecture() -> Result<Vec<Finding>> {
    use std::collections::{BTreeMap, BTreeSet};
    let idx = index::Index::load()?;
    let mut findings = Vec::new();
    let mut graph = BTreeMap::<String, BTreeSet<String>>::new();
    for (path, entry) in &idx.files {
        let Some((module, _)) = path
            .strip_prefix("src/modules/")
            .and_then(|s| s.split_once('/'))
        else {
            continue;
        };
        for dep in &entry.dependencies {
            let mut paths = vec![];
            module_imports(&syn::parse_str(dep)?, "", &mut paths);
            for target in paths {
                let Some(rest) = target.strip_prefix("crate::modules::") else {
                    continue;
                };
                let mut parts = rest.split("::");
                let name = parts.next().unwrap();
                if name == module {
                    continue;
                }
                graph.entry(module.into()).or_default().insert(name.into());
                if parts.next() == Some("controller") {
                    findings.push(Finding::new(
                        path,
                        0,
                        "ARCH003",
                        "proven",
                        "cross-module controller import",
                    ))
                }
            }
        }
    }
    for module in graph.keys() {
        let mut stack = vec![module];
        let mut seen = BTreeSet::new();
        while let Some(v) = stack.pop() {
            if !seen.insert(v) {
                continue;
            }
            if let Some(targets) = graph.get(v) {
                if targets.contains(module) {
                    findings.push(Finding::new(
                        &format!("src/modules/{module}"),
                        0,
                        "ARCH002",
                        "heuristic",
                        "cycle in explicit module imports",
                    ));
                    break;
                }
                stack.extend(targets)
            }
        }
    }
    Ok(findings)
}
