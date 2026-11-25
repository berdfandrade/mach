use std::{path::Path, process::Command};
use testcontainers::{
    GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};

#[tokio::test]
#[ignore = "requires a local Docker daemon and postgres:16-alpine"]
async fn generated_application() -> anyhow::Result<()> {
    let password = uuid::Uuid::new_v4().to_string();
    let db = GenericImage::new("postgres", "16-alpine")
        .with_exposed_port(5432.tcp())
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_env_var("POSTGRES_PASSWORD", &password)
        .with_startup_timeout(std::time::Duration::from_secs(300))
        .start()
        .await?;
    let port = db.get_host_port_ipv4(5432).await?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let status = Command::new("python3")
        .arg(root.join("tests/workflow.py"))
        .arg(env!("CARGO_BIN_EXE_mach"))
        .env(
            "DATABASE_URL",
            format!("postgres://postgres:{password}@127.0.0.1:{port}/postgres"),
        )
        .env("CARGO_TARGET_DIR", root.join("target"))
        .status()?;
    anyhow::ensure!(status.success(), "generated application workflow failed");
    Ok(())
}
