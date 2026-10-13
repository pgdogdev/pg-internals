use std::{env, fs, path::PathBuf, process::Command};

const URL: &str = "https://github.com/postgres/postgres.git";
const REVISION: &str = "9e7d152488f197e96f171f0714459c227c3f7693";

/// Fetch only the pinned commit and reuse it across build-script reruns.
pub fn prepare() -> PathBuf {
    println!("cargo:rerun-if-changed=build/source.rs");
    let source = PathBuf::from(env::var_os("OUT_DIR").expect("missing output directory"))
        .join(format!("postgres-source-{REVISION}"));
    let ready = source.join(".pg-internals-ready");
    if !ready.is_file() {
        fs::create_dir_all(&source).expect("cannot create PostgreSQL source directory");
        run(Command::new("git")
            .current_dir(&source)
            .args(["init", "--quiet"]));
        run(Command::new("git").current_dir(&source).args([
            "fetch",
            "--depth",
            "1",
            "--no-tags",
            URL,
            REVISION,
        ]));
        run(Command::new("git")
            .current_dir(&source)
            .args(["checkout", "--quiet", "--detach", REVISION]));
        fs::write(&ready, REVISION).expect("cannot mark PostgreSQL source ready");
    }
    source
        .canonicalize()
        .expect("cannot locate PostgreSQL source")
}

fn run(command: &mut Command) {
    let output = command
        .output()
        .expect("cannot run Git: Git is required to fetch PostgreSQL source");
    assert!(
        output.status.success(),
        "{command:?} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}
