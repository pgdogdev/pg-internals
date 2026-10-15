use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use sha2::{Digest, Sha256};

#[allow(dead_code)]
#[path = "../../pg_internals/build/configure.rs"]
mod configure;
#[path = "../../pg_internals/build/guc_sources.rs"]
mod guc_sources;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const URL: &str = "https://github.com/postgres/postgres.git";
const REVISION: &str = include_str!("../postgres-revision");
const HELP: &str = "Usage: cargo run --manifest-path xtask/Cargo.toml -- vendor-postgres [--source CHECKOUT] [--output DIRECTORY] [--check]\n\n--source uses an existing Git repository containing the pinned commit.\n--check regenerates and compares without changing the destination.\nCLANG_PATH selects the Clang executable used for include discovery.";

fn main() -> ExitCode {
    match vendor() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("vendoring failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn vendor() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut destination = root.join("pg_internals/vendor/postgres");
    let mut local = None;
    let mut check = false;
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("vendor-postgres") => (),
        Some("--help" | "-h") => {
            println!("{HELP}");
            return Ok(());
        }
        _ => return Err(HELP.into()),
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source" => {
                local = Some(
                    PathBuf::from(args.next().ok_or("missing --source path")?).canonicalize()?,
                )
            }
            "--output" => destination = PathBuf::from(args.next().ok_or("missing --output path")?),
            "--check" => check = true,
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(());
            }
            _ => return Err(format!("unknown argument {arg}\n{HELP}").into()),
        }
    }
    let revision = REVISION.trim();
    if revision.len() != 40 || !revision.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("postgres-revision must contain a full Git commit hash".into());
    }
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    let build = temp.path().join("build");
    let stage = temp.path().join("vendor");
    eprintln!("Preparing PostgreSQL {revision}");
    if let Some(local) = local {
        run(Command::new("git")
            .args(["clone", "--quiet", "--no-checkout", "--shared"])
            .arg(local)
            .arg(&source))?;
    } else {
        fs::create_dir(&source)?;
        run(Command::new("git").arg("init").arg("--quiet").arg(&source))?;
        run(Command::new("git").current_dir(&source).args([
            "fetch",
            "--depth=1",
            "--no-tags",
            URL,
            revision,
        ]))?;
    }
    run(Command::new("git")
        .current_dir(&source)
        .args(["checkout", "--quiet", "--detach", revision]))?;
    let head = run(Command::new("git")
        .current_dir(&source)
        .args(["rev-parse", "HEAD"]))?;
    if head.trim() != revision {
        return Err("PostgreSQL revision mismatch".into());
    }

    eprintln!("Generating PostgreSQL headers");
    configure::run(&source, &build);
    run(Command::new("make")
        .arg("-C")
        .arg(build.join("src/backend"))
        .arg("generated-headers"))?;

    let catalog = source.join("src/backend/catalog");
    let records = run(Command::new("perl")
        .arg("-I")
        .arg(&catalog)
        .arg(root.join("pg_internals/build/guc.pl"))
        .arg(source.join("src/backend/utils/misc/guc_parameters.dat")))?;
    let records: Vec<BTreeMap<String, String>> = serde_json::from_str(&records)?;
    let wanted = records
        .iter()
        .filter(|fields| !fields.contains_key("ifdef"))
        .filter_map(|fields| fields.get("options"))
        .map(|reference| guc_sources::table_reference(reference).0)
        .collect();
    let mut units = guc_sources::select(&source, &wanted);
    units.extend([
        root.join("pg_internals/build/constants.h"),
        root.join("pg_internals/build/guc_parse.c"),
        // The build extracts parser definitions from this source at compile time.
        source.join("src/backend/utils/misc/guc.c"),
    ]);

    let mut files = BTreeMap::new();
    // Retain every header, including inactive platform branches. Never derive
    // a portable header inventory solely from one host's preprocessor output.
    for path in walk(&source.join("src/include"))? {
        add(&mut files, &source, &path)?;
    }
    for path in walk(&source.join("src/backend"))? {
        if extension(&path, "h") {
            add(&mut files, &source, &path)?;
        }
    }
    // configure checks heaptuple.c and links platform-specific IPC sources.
    for name in [
        "COPYRIGHT",
        "configure",
        "Makefile",
        "GNUmakefile.in",
        "src/Makefile.global.in",
        "src/interfaces/ecpg/include/ecpg_config.h.in",
        "src/backend/access/common/heaptuple.c",
        "src/backend/catalog/Catalog.pm",
        "src/backend/utils/misc/guc_parameters.dat",
    ] {
        add(&mut files, &source, &source.join(name))?;
    }
    for directory in [
        "config",
        "src/template",
        "src/makefiles",
        "src/backend/port",
    ] {
        for path in walk(&source.join(directory))? {
            add(&mut files, &source, &path)?;
        }
    }
    // Generated headers are independent of the host except for configuration
    // headers. probes.h is the default (DTrace disabled) generated stub.
    let mut generated = BTreeSet::new();
    for path in walk(&build.join("src/include"))? {
        if (extension(&path, "h") || extension(&path, "c")) && !configuration_header(&path) {
            let relative = add(&mut files, &build, &path)?;
            generated.insert(relative);
        }
    }
    eprintln!("Discovering includes for {} translation units", units.len());
    for unit in &units {
        for dependency in dependencies(unit, &source, &build)? {
            if dependency.starts_with(&source) {
                add(&mut files, &source, &dependency)?;
            } else if dependency.starts_with(&build) && !configuration_header(&dependency) {
                let relative = add(&mut files, &build, &dependency)?;
                generated.insert(relative);
            }
        }
    }

    // Upstream ignores its generated headers. Keeping these rules would omit
    // exactly those files from Git and from the packaged Rust crate.
    files.retain(|path, _| {
        !matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some(".gitignore" | ".gitattributes")
        )
    });
    fs::create_dir(&stage)?;
    for (relative, input) in &files {
        let target = stage.join(relative);
        fs::create_dir_all(target.parent().unwrap())?;
        if generated.contains(relative) {
            // Bison can put absolute paths in #line directives. Only generated
            // output is normalized; upstream source bytes are never rewritten.
            let text = fs::read_to_string(input)?
                .replace(&format!("{}/", source.display()), "")
                .replace(&format!("{}/", build.display()), "");
            fs::write(&target, text)?;
        } else {
            fs::copy(input, &target)?;
        }
    }
    let manifest = serde_json::json!({
        "format": 1,
        "repository": URL,
        "revision": revision,
        "configuration": "PostgreSQL defaults; host configuration headers excluded; DTrace disabled",
        "generated": generated,
        "sha256": inventory(&stage)?,
    });
    fs::write(
        stage.join("MANIFEST.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;

    // A second preprocessing pass must resolve everything using the subset.
    // Keep only configure's host headers in a fresh include directory so the
    // original generated tree cannot accidentally hide an omitted dependency.
    let validation = temp.path().join("validation");
    fs::create_dir_all(validation.join("src/include"))?;
    for name in ["pg_config.h", "pg_config_os.h"] {
        fs::copy(
            build.join("src/include").join(name),
            validation.join("src/include").join(name),
        )?;
    }
    for unit in &units {
        let unit = unit
            .strip_prefix(&source)
            .map(|path| stage.join(path))
            .unwrap_or_else(|_| unit.clone());
        dependencies(&unit, &stage, &validation)?;
    }

    if check {
        let expected = inventory(&stage)?;
        let actual = inventory(&destination)?;
        let differences: BTreeSet<_> = expected
            .keys()
            .chain(actual.keys())
            .filter(|path| expected.get(*path) != actual.get(*path))
            .collect();
        if !differences.is_empty() {
            return Err(format!(
                "vendored files differ:\n{}",
                differences
                    .into_iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
            .into());
        }
        eprintln!("Vendored PostgreSQL is up to date ({} files)", files.len());
    } else {
        install(&stage, &destination)?;
        eprintln!(
            "Vendored {} files into {}",
            files.len(),
            destination.display()
        );
    }
    Ok(())
}

fn configuration_header(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("pg_config.h" | "pg_config_os.h" | "ecpg_config.h")
    )
}

fn extension(path: &Path, extension: &str) -> bool {
    path.extension().is_some_and(|ext| ext == extension)
}

fn run(command: &mut Command) -> Result<String> {
    let output = command
        .output()
        .map_err(|error| format!("{command:?}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{command:?} failed ({}):\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn walk(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            paths.extend(walk(&entry.path())?);
        } else if entry.path().is_file() {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn add(files: &mut BTreeMap<PathBuf, PathBuf>, root: &Path, path: &Path) -> Result<PathBuf> {
    // Preserve the include spelling even when the generated file is a symlink.
    let relative = path.strip_prefix(root)?.to_path_buf();
    if relative
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(format!("invalid vendored path: {}", path.display()).into());
    }
    if !path.is_file() {
        return Err(format!("missing input: {}", path.display()).into());
    }
    files.insert(relative.clone(), path.to_path_buf());
    Ok(relative)
}

fn dependencies(unit: &Path, source: &Path, build: &Path) -> Result<Vec<PathBuf>> {
    let output = run(
        Command::new(env::var_os("CLANG_PATH").unwrap_or_else(|| "clang".into()))
            .args(["-x", "c", "-M", "-MT", "dependencies", "-D_GNU_SOURCE"])
            .arg("-I")
            .arg(build.join("src/include"))
            .arg("-I")
            .arg(source.join("src/include"))
            .arg("-I")
            .arg(source.join("src/backend"))
            .arg(unit),
    )?;
    parse_dependencies(&output)
}

/// Decode Clang's single Make rule, including escaped spaces and continuations.
fn parse_dependencies(output: &str) -> Result<Vec<PathBuf>> {
    let (_, prerequisites) = output
        .split_once(':')
        .ok_or("invalid Clang dependency output")?;
    let joined = prerequisites.replace("\\\n", "");
    let mut chars = joined.chars().peekable();
    let mut paths = Vec::new();
    let mut word = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => word.push(chars.next().ok_or("trailing dependency escape")?),
            '$' if chars.peek() == Some(&'$') => {
                chars.next();
                word.push('$');
            }
            c if c.is_whitespace() => {
                if !word.is_empty() {
                    paths.push(PathBuf::from(std::mem::take(&mut word)));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        paths.push(PathBuf::from(word));
    }
    Ok(paths)
}

fn inventory(directory: &Path) -> Result<BTreeMap<PathBuf, String>> {
    walk(directory)?
        .into_iter()
        .map(|path| {
            let hash = format!("{:x}", Sha256::digest(fs::read(&path)?));
            Ok((path.strip_prefix(directory)?.to_path_buf(), hash))
        })
        .collect()
}

fn install(stage: &Path, destination: &Path) -> Result<()> {
    // Refuse to replace an unrelated directory. Stage on the destination's
    // filesystem, then rename; generation failures leave the old tree intact.
    if destination.exists() && !destination.join("MANIFEST.json").is_file() {
        return Err(format!(
            "refusing to replace {} without MANIFEST.json",
            destination.display()
        )
        .into());
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let transaction = tempfile::tempdir_in(parent)?;
    let new = transaction.path().join("new");
    fs::create_dir(&new)?;
    for file in walk(stage)? {
        let target = new.join(file.strip_prefix(stage)?);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(file, target)?;
    }
    let backup = transaction.path().join("old");
    if destination.exists() {
        fs::rename(destination, &backup)?;
    }
    if let Err(error) = fs::rename(&new, destination) {
        if backup.exists()
            && let Err(restore) = fs::rename(&backup, destination)
        {
            let recovery = transaction.keep();
            return Err(format!(
                "install failed: {error}; restore failed: {restore}; previous tree retained in {}",
                recovery.join("old").display()
            )
            .into());
        }
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_dependencies_preserve_escaped_paths() {
        let paths = parse_dependencies(
            "dependencies: /tmp/a\\ b.h \\\n /tmp/c\\#d.h /tmp/e$$f.h /tmp/g\\\\h.h\n",
        )
        .unwrap();
        assert_eq!(
            paths,
            ["/tmp/a b.h", "/tmp/c#d.h", "/tmp/e$f.h", "/tmp/g\\h.h"].map(PathBuf::from)
        );
        assert!(parse_dependencies("broken").is_err());
    }

    #[test]
    fn replacement_removes_stale_files_and_protects_unrelated_directories() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let stage = temp.path().join("stage");
        fs::create_dir(&old).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::write(old.join("stale"), "old").unwrap();
        assert!(install(&stage, &old).is_err());
        assert!(old.join("stale").is_file());
        fs::write(old.join("MANIFEST.json"), "{}").unwrap();
        fs::write(stage.join("MANIFEST.json"), "{}").unwrap();
        fs::write(stage.join("fresh"), "new").unwrap();
        install(&stage, &old).unwrap();
        assert!(!old.join("stale").exists());
        assert_eq!(fs::read_to_string(old.join("fresh")).unwrap(), "new");
    }
}
