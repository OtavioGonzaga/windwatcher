use std::{fs, path::Path, process::Command};

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("builder directory");
    for entry in fs::read_dir(source).expect("source directory") {
        let entry = entry.expect("source entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("source file type").is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("builder input");
        }
    }
}

#[test]
fn container_builder_copy_inputs_form_a_complete_locked_workspace() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let containerfile =
        fs::read_to_string(repository.join("Containerfile")).expect("Containerfile");
    let builder = tempfile::tempdir().expect("isolated builder filesystem");

    // Reproduce the builder's COPY inputs, not the whole checkout: otherwise a
    // newly added workspace member can pass local Cargo checks but break OCI.
    for line in containerfile
        .lines()
        .take_while(|line| !line.starts_with("RUN "))
    {
        let Some(copy) = line.strip_prefix("COPY ") else {
            continue;
        };
        let arguments: Vec<_> = copy.split_whitespace().collect();
        assert!(
            arguments.len() >= 2,
            "expected COPY sources and destination"
        );
        let destination = builder
            .path()
            .join(arguments.last().expect("COPY destination"));
        fs::create_dir_all(&destination).expect("COPY destination");
        for source in &arguments[..arguments.len() - 1] {
            let source = repository.join(source);
            if source.is_dir() {
                copy_directory(&source, &destination);
            } else {
                fs::copy(
                    &source,
                    destination.join(source.file_name().expect("COPY filename")),
                )
                .expect("COPY file");
            }
        }
    }

    let output = Command::new("cargo")
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
        ])
        .env("CARGO_BUILD_JOBS", "4")
        .current_dir(builder.path())
        .output()
        .expect("builder Cargo metadata");
    assert!(
        output.status.success(),
        "builder workspace is incomplete: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Cargo metadata JSON");
    assert!(
        metadata["packages"]
            .as_array()
            .expect("packages")
            .iter()
            .any(|package| package["name"] == "xtask")
    );
}
