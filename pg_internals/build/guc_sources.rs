use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub fn table_reference(reference: &str) -> (String, usize) {
    let expression: syn::Expr = syn::parse_str(reference)
        .unwrap_or_else(|_| panic!("unsupported GUC enum table reference {reference}"));
    let (table, offset) = match &expression {
        syn::Expr::Binary(binary) if matches!(binary.op, syn::BinOp::Add(_)) => {
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(offset),
                ..
            }) = binary.right.as_ref()
            else {
                panic!("unsupported GUC enum table offset {reference}");
            };
            (
                binary.left.as_ref(),
                offset.base10_parse().expect("invalid enum table offset"),
            )
        }
        table => (table, 0),
    };
    let syn::Expr::Path(table) = table else {
        panic!("unsupported GUC enum table reference {reference}");
    };
    (
        table
            .path
            .get_ident()
            .expect("invalid enum table name")
            .to_string(),
        offset,
    )
}

fn find_sources(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("cannot list PostgreSQL sources") {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            find_sources(&entry.path(), files);
        } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "c") {
            files.push(entry.path());
        }
    }
}

/// Select translation units identically for builds and source vendoring.
pub fn select(source: &Path, wanted: &BTreeSet<String>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    find_sources(&source.join("src/backend"), &mut files);
    files.sort();
    files.retain(|file| {
        let text = fs::read_to_string(file).expect("cannot read PostgreSQL C source");
        text.contains("config_enum_entry") && wanted.iter().any(|name| text.contains(name))
    });
    files
}
