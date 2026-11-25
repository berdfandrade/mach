use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path},
    process::{Command, Stdio},
};
use tempfile::NamedTempFile;

pub fn checked(path: &Path) -> Result<()> {
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))),
        "invalid project path"
    );
    for p in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        if let Ok(m) = p.symlink_metadata() {
            ensure!(!m.is_symlink(), "refusing symlink {}", p.display())
        }
    }
    Ok(())
}
pub fn atomic(path: &Path, data: &[u8]) -> Result<()> {
    checked(path)?;
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent)?;
    let mut temp = NamedTempFile::new_in(parent)?;
    if let Ok(m) = path.metadata() {
        temp.as_file().set_permissions(m.permissions())?
    }
    temp.write_all(data)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    File::open(if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    })?
    .sync_all()?;
    Ok(())
}
pub fn lock() -> Result<File> {
    checked(Path::new(".mach/lock"))?;
    fs::create_dir_all(".mach")?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(".mach/lock")?;
    lock.try_lock_exclusive()
        .context("another MACH command owns the project lock")?;
    if Path::new(JOURNAL).exists() {
        recover()?
    }
    Ok(lock)
}
const JOURNAL: &str = ".mach/journal.json";
#[derive(Serialize, Deserialize)]
struct Change {
    before: Option<String>,
    after: String,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Plan(BTreeMap<String, Change>);
fn read(path: &str) -> Result<Option<String>> {
    checked(Path::new(path))?;
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
impl Plan {
    pub fn stage_from(&mut self, path: &str, before: Option<String>, after: String) -> Result<()> {
        ensure!(before == read(path)?, "concurrent edit: {path}");
        if before.as_ref() != Some(&after) {
            self.0.insert(path.into(), Change { before, after });
        }
        Ok(())
    }
    pub fn create(&mut self, path: &str, data: String) -> Result<()> {
        match read(path)? {
            None => self.stage_from(path, None, data),
            Some(existing) if existing == data => Ok(()),
            Some(_) => bail!("existing file differs: {path}"),
        }
    }
    pub fn paths(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
    pub fn apply(self, verify: bool) -> Result<()> {
        if self.0.is_empty() {
            return Ok(());
        }
        for (p, c) in &self.0 {
            ensure!(read(p)? == c.before, "concurrent edit: {p}");
            if p.ends_with(".rs") {
                syn::parse_file(&c.after).with_context(|| format!("invalid syntax in {p}"))?;
            }
        }
        atomic(Path::new(JOURNAL), &serde_json::to_vec(&self)?)?;
        let result = (|| {
            for (p, c) in &self.0 {
                ensure!(read(p)? == c.before, "concurrent edit: {p}");
                atomic(Path::new(p), c.after.as_bytes())?
            }
            if verify {
                let ok = Command::new("cargo")
                    .args(["check", "--all-targets", "--quiet"])
                    .stderr(Stdio::null())
                    .status()?
                    .success();
                ensure!(
                    ok,
                    "cargo check failed; run cargo check for project diagnostics"
                )
            }
            Ok(())
        })();
        if let Err(e) = result {
            recover()?;
            return Err(e);
        }
        fs::remove_file(JOURNAL)?;
        Ok(())
    }
}
fn recover() -> Result<()> {
    checked(Path::new(JOURNAL))?;
    let plan: Plan = serde_json::from_str(&fs::read_to_string(JOURNAL)?)?;
    for (p, c) in plan.0.iter().rev() {
        let current = read(p)?;
        if current == c.before {
            continue;
        }
        ensure!(
            current.as_ref() == Some(&c.after),
            "manual recovery required: concurrently modified {p}; review .mach/journal.json"
        );
        if let Some(v) = &c.before {
            atomic(Path::new(p), v.as_bytes())?
        } else {
            fs::remove_file(p)?
        }
    }
    fs::remove_file(JOURNAL)?;
    Ok(())
}
pub fn format(source: &str) -> Result<String> {
    let mut p = Command::new("rustfmt")
        .args(["--edition", "2024", "--config", "skip_children=true"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    p.stdin.take().unwrap().write_all(source.as_bytes())?;
    let out = p.wait_with_output()?;
    ensure!(out.status.success(), "rustfmt failed");
    Ok(String::from_utf8(out.stdout)?)
}
