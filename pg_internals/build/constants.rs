use std::{collections::BTreeMap, env, fs, path::PathBuf, process::Command};

use super::guc;

#[derive(Debug)]
struct ConstantNames {
    aliases: BTreeMap<String, String>,
}

impl bindgen::callbacks::ParseCallbacks for ConstantNames {
    fn enum_variant_behavior(
        &self,
        _enum_name: Option<&str>,
        name: &str,
        _value: bindgen::callbacks::EnumVariantValue,
    ) -> Option<bindgen::callbacks::EnumVariantCustomBehavior> {
        self.aliases
            .contains_key(name)
            .then_some(bindgen::callbacks::EnumVariantCustomBehavior::Hide)
    }

    fn item_name(&self, item: bindgen::callbacks::ItemInfo<'_>) -> Option<String> {
        item.name.strip_prefix("PG_INTERNALS_").map(str::to_owned)
    }

    fn enum_variant_name(
        &self,
        _enum_name: Option<&str>,
        name: &str,
        _value: bindgen::callbacks::EnumVariantValue,
    ) -> Option<String> {
        name.strip_prefix("PG_INTERNALS_").map(str::to_owned)
    }
}

pub fn build() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let source = root.join("../postgres").canonicalize().unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let postgres = output.join("postgres");
    let wrapper = root.join("build/constants.h");

    // Backend headers also depend on generated catalog, node, and error headers.
    for path in ["src/include", "src/backend", "src/Makefile.global.in"] {
        println!("cargo:rerun-if-changed={}", source.join(path).display());
    }
    println!("cargo:rerun-if-changed={}", wrapper.display());
    println!("cargo:rerun-if-env-changed=LIBCLANG_PATH");
    let generated = Command::new("make")
        .arg("-C")
        .arg(postgres.join("src/backend"))
        .arg("generated-headers")
        .output()
        .expect("cannot generate PostgreSQL headers: make is required");
    let log = postgres.join("generated-headers.log");
    let mut transcript = generated.stdout;
    transcript.extend_from_slice(&generated.stderr);
    fs::write(&log, transcript).expect("cannot write the header generation log");
    assert!(
        generated.status.success(),
        "PostgreSQL header generation failed ({}). See {}.\n{}",
        generated.status,
        log.display(),
        String::from_utf8_lossy(&generated.stderr)
    );

    let macro_output = output.join("clang-macros");
    fs::create_dir_all(&macro_output).expect("cannot create the Clang macro directory");
    let mut aliases = BTreeMap::from([
        ("INT_MIN".to_owned(), "int".to_owned()),
        ("INT_MAX".to_owned(), "int".to_owned()),
        ("DBL_MAX".to_owned(), "real".to_owned()),
        ("SIZE_MAX".to_owned(), "size".to_owned()),
    ]);
    for fields in guc::parameters().values() {
        if fields.contains_key("ifdef") {
            continue;
        }
        let kind = &fields["type"];
        for field in ["boot_val", "min", "max"] {
            let Some(value) = fields.get(field) else {
                continue;
            };
            let value = guc::without_comments(value);
            let Ok((_, names)) = guc::arithmetic(&value) else {
                continue;
            };
            // PostgreSQL represents enum values with integers.
            let kind = if kind == "enum" { "int" } else { kind.as_str() };
            for name in names {
                if name == "NULL" {
                    continue;
                }
                if let Some(previous) = aliases.insert(name.clone(), kind.to_owned()) {
                    assert_eq!(previous, kind, "conflicting GUC types for {name}");
                }
            }
        }
    }

    // Force Clang to evaluate named values with the types used by GUCs, including
    // enum members and macros referencing enum members. Preserve the public names.
    let mut header = String::new();
    // Match the Makefile's PG_KRB_SRVTAB definition rather than guc_tables.c's fallback.
    let makefile = fs::read_to_string(postgres.join("src/Makefile.global"))
        .expect("cannot read configured PostgreSQL build settings");
    let krb_srvtab = makefile
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "krb_srvtab").then(|| value.trim())
        })
        .expect("missing krb_srvtab build setting");
    header.push_str(&format!("#define PG_KRB_SRVTAB {krb_srvtab:?}\n"));
    header.push_str(&fs::read_to_string(&wrapper).expect("cannot read constants wrapper"));
    for (name, kind) in &aliases {
        match kind.as_str() {
            "string" => header.push_str(&format!("\n#define PG_INTERNALS_{name} {name}\n")),
            "size" => header.push_str(&format!("\nenum {{ PG_INTERNALS_{name} = {name} }};\n")),
            _ => {
                let c_type = match kind.as_str() {
                    "int" => "int",
                    "real" => "double",
                    "bool" => "bool",
                    _ => panic!("unsupported GUC type {kind}"),
                };
                header.push_str(&format!(
                    "\nstatic const {c_type} PG_INTERNALS_{name} = {name};\n"
                ));
            }
        }
    }
    let generated_wrapper = output.join("constants-wrapper.h");
    fs::write(&generated_wrapper, header).expect("cannot write constants wrapper");
    let mut builder = bindgen::Builder::default()
        .header(generated_wrapper.to_str().expect("non-UTF-8 wrapper path"))
        .clang_arg(format!("-I{}", postgres.join("src/include").display()))
        .clang_arg(format!("-I{}", source.join("src/include").display()))
        .clang_arg(format!("-I{}", source.join("src/backend").display()))
        .clang_arg("-D_GNU_SOURCE")
        .allowlist_var("(DEFAULT|MIN|MAX|PG|PGC|GUC|XACT|WAL|ARCHIVE_MODE)_[A-Z0-9_]+")
        .allowlist_var("PG_INTERNALS_.*")
        .allowlist_var("BLCKSZ|XLOG_BLCKSZ|RELSEG_SIZE|NAMEDATALEN|INDEX_MAX_KEYS|FUNC_MAX_ARGS")
        .allowlist_var("INT_MIN|INT_MAX|DBL_MAX|SIZE_MAX")
        .allowlist_type("ArchiveMode|WalLevel|WalCompression|GucContext|GucSource")
        .allowlist_recursively(false)
        .with_codegen_config(bindgen::CodegenConfig::VARS | bindgen::CodegenConfig::TYPES)
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate_comments(false)
        .clang_macro_fallback()
        .clang_macro_fallback_build_dir(macro_output);
    for name in aliases.keys() {
        builder = builder.blocklist_item(name);
    }
    builder
        .parse_callbacks(Box::new(ConstantNames { aliases }))
        .generate()
        .expect("cannot generate PostgreSQL constants with bindgen")
        .write_to_file(output.join("constants.rs"))
        .expect("cannot write generated PostgreSQL constants");
}
