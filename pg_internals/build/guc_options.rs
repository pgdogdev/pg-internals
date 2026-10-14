use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::{CStr, CString},
    path::{Path, PathBuf},
    ptr,
};

use clang_sys::*;

use super::{
    guc::Fields,
    guc_sources::{select, table_reference},
};

/// Read enum strings from Clang's preprocessed C initializer ASTs.
pub fn parse(source: &Path, entries: &BTreeMap<String, Fields>) -> BTreeMap<String, Vec<String>> {
    let references: BTreeMap<_, _> = entries
        .values()
        .filter(|fields| !fields.contains_key("ifdef"))
        .filter_map(|fields| fields.get("options"))
        .map(|reference| (reference.clone(), table_reference(reference)))
        .collect();
    let wanted: BTreeSet<_> = references.values().map(|(name, _)| name.clone()).collect();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    // Use the same headers and feature definitions as the constants bindgen step.
    let mut arguments = vec![
        "-x".to_owned(),
        "c".to_owned(),
        "-D_GNU_SOURCE".to_owned(),
        format!("-I{}", output.join("postgres/src/include").display()),
        format!("-I{}", source.join("src/include").display()),
        format!("-I{}", source.join("src/backend").display()),
    ];
    // Like bindgen, discover Clang's system headers (including stdalign.h).
    println!("cargo:rerun-if-env-changed=CLANG_PATH");
    if let Some(clang) = clang_sys::support::Clang::find(None, &[]) {
        for path in clang.c_search_paths.unwrap_or_default() {
            arguments.push("-isystem".to_owned());
            arguments.push(path.to_str().expect("non-UTF-8 include path").to_owned());
        }
    }
    let arguments: Vec<_> = arguments
        .into_iter()
        .map(|argument| CString::new(argument).unwrap())
        .collect();

    clang_sys::load().expect("cannot load libclang to read GUC enum options");
    let mut options = BTreeMap::new();
    for file in select(source, &wanted) {
        println!("cargo:rerun-if-changed={}", file.display());
        let unit = TranslationUnit::parse(&file, &arguments);
        // SAFETY: all cursors below are used before their translation unit is dropped.
        for declaration in children(unsafe { clang_getTranslationUnitCursor(unit.unit) }) {
            if declaration.kind != CXCursor_VarDecl {
                continue;
            }
            let name = clang_string(unsafe { clang_getCursorSpelling(declaration) });
            if !wanted.contains(&name) {
                continue;
            }
            let Some(initializer) = children(declaration)
                .into_iter()
                .find(|child| child.kind == CXCursor_InitListExpr)
            else {
                continue; // An extern declaration, not the array definition.
            };
            let rows = children(initializer);
            let mut values = Vec::new();
            let mut terminated = false;
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(
                    row.kind, CXCursor_InitListExpr,
                    "invalid enum row in {name}"
                );
                let fields = children(*row);
                let first = *fields.first().expect("empty enum row");
                if let Some(value) = string_literal(first) {
                    values.push(value);
                } else {
                    // PostgreSQL terminates each table with {NULL, 0, false}.
                    assert_eq!(index + 1, rows.len(), "non-string option in {name}");
                    terminated = true;
                }
            }
            assert!(
                terminated && !values.is_empty(),
                "invalid enum table {name}"
            );
            assert!(
                options.insert(name.clone(), values).is_none(),
                "duplicate enum table {name}"
            );
        }
    }
    references
        .into_iter()
        .map(|(reference, (name, offset))| {
            let values = options
                .get(&name)
                .unwrap_or_else(|| panic!("cannot find GUC enum table {name}"));
            assert!(
                offset < values.len(),
                "invalid GUC enum table offset: {reference}"
            );
            (reference, values[offset..].to_vec())
        })
        .collect()
}

// These helpers are private: their cursors and strings always belong to a live TU.
fn children(cursor: CXCursor) -> Vec<CXCursor> {
    extern "C" fn collect(cursor: CXCursor, _: CXCursor, data: CXClientData) -> CXChildVisitResult {
        // SAFETY: clang_visitChildren calls synchronously with our Vec pointer.
        unsafe { &mut *data.cast::<Vec<CXCursor>>() }.push(cursor);
        CXChildVisit_Continue
    }
    let mut children = Vec::<CXCursor>::new();
    unsafe { clang_visitChildren(cursor, collect, ptr::from_mut(&mut children).cast()) };
    children
}

fn clang_string(string: CXString) -> String {
    // SAFETY: copy the Clang-owned string before releasing it.
    let bytes = unsafe {
        let value = clang_getCString(string);
        let bytes = if value.is_null() {
            Vec::new()
        } else {
            CStr::from_ptr(value).to_bytes().to_vec()
        };
        clang_disposeString(string);
        bytes
    };
    String::from_utf8(bytes).expect("non-UTF-8 Clang string")
}

fn string_literal(cursor: CXCursor) -> Option<String> {
    if cursor.kind != CXCursor_StringLiteral {
        return children(cursor).into_iter().find_map(string_literal);
    }
    // Clang expands macros and concatenates adjacent literals in the AST. Decode
    // its quoted spelling with the same C string decoder used for GUC defaults.
    let spelling = clang_string(unsafe { clang_getCursorSpelling(cursor) });
    Some(
        super::guc::c_string_literal(&spelling)
            .unwrap_or_else(|| panic!("unsupported GUC enum string {spelling}")),
    )
}

struct TranslationUnit {
    index: CXIndex,
    unit: CXTranslationUnit,
}

impl TranslationUnit {
    fn parse(file: &Path, arguments: &[CString]) -> Self {
        let filename = CString::new(file.to_str().expect("non-UTF-8 C source path")).unwrap();
        let arguments: Vec<_> = arguments.iter().map(|arg| arg.as_ptr()).collect();
        // SAFETY: filenames and arguments live through parsing; ownership of the
        // index and resulting TU is retained here and released in Drop.
        let unit = unsafe {
            let index = clang_createIndex(0, 0);
            assert!(!index.is_null(), "cannot create Clang index");
            let unit = clang_parseTranslationUnit(
                index,
                filename.as_ptr(),
                arguments.as_ptr(),
                arguments.len() as i32,
                ptr::null_mut(),
                0,
                CXTranslationUnit_SkipFunctionBodies,
            );
            if unit.is_null() {
                clang_disposeIndex(index);
                panic!("cannot parse {}", file.display());
            }
            Self { index, unit }
        };
        let mut errors = Vec::new();
        unsafe {
            for index in 0..clang_getNumDiagnostics(unit.unit) {
                let diagnostic = clang_getDiagnostic(unit.unit, index);
                if clang_getDiagnosticSeverity(diagnostic) >= CXDiagnostic_Error {
                    errors.push(clang_string(clang_formatDiagnostic(
                        diagnostic,
                        clang_defaultDiagnosticDisplayOptions(),
                    )));
                }
                clang_disposeDiagnostic(diagnostic);
            }
        }
        assert!(
            errors.is_empty(),
            "cannot parse {}:\n{}",
            file.display(),
            errors.join("\n")
        );
        unit
    }
}

impl Drop for TranslationUnit {
    fn drop(&mut self) {
        unsafe {
            clang_disposeTranslationUnit(self.unit);
            clang_disposeIndex(self.index);
        }
    }
}
