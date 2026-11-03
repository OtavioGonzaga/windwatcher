use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Command};

pub(crate) struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// Small seam for testing external systems without publishing anything.
pub(crate) trait Runner {
    fn run(&self, root: &Path, program: &str, args: &[&str]) -> Result<Output>;
}

pub(crate) struct Process;
impl Runner for Process {
    fn run(&self, root: &Path, program: &str, args: &[&str]) -> Result<Output> {
        let mut command = Command::new(program);
        command.args(args).current_dir(root);
        // Registry credentials are needed by Cargo publication only. Never echo
        // the environment or command output containing credentials.
        if !(program == "cargo" && args.first() == Some(&"publish")) {
            command.env_remove("CARGO_REGISTRY_TOKEN");
            command.env_remove("CRATES_IO_TOKEN");
        }
        if program != "gh" {
            command.env_remove("GH_TOKEN").env_remove("GITHUB_TOKEN");
        }
        if program == "cargo" && std::env::var_os("CARGO_BUILD_JOBS").is_none() {
            command.env("CARGO_BUILD_JOBS", "4");
        }
        let output = command
            .output()
            .with_context(|| format!("starting {program}"))?;
        Ok(Output {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

pub(crate) fn bytes(r: &dyn Runner, root: &Path, program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let out = r.run(root, program, args)?;
    ensure!(out.success, "{program} failed: {}", out.stderr);
    Ok(out.stdout)
}

pub(crate) fn text(r: &dyn Runner, root: &Path, program: &str, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(bytes(r, root, program, args)?)?
        .trim()
        .to_owned())
}
