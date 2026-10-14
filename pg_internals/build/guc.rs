use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

pub fn parameters(source: &Path) -> BTreeMap<String, Fields> {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let backend = source.join("src/backend");
    let input = backend.join("utils/misc/guc_parameters.dat");
    let catalog = backend.join("catalog");
    let wrapper = root.join("build/guc.pl");
    for path in [&input, &wrapper, &catalog.join("Catalog.pm")] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let parsed = Command::new("perl")
        .arg("-I")
        .arg(&catalog)
        .arg(&wrapper)
        .arg(&input)
        .output()
        .expect("cannot run the GUC parser: Perl is required to build pg_internals");
    assert!(
        parsed.status.success(),
        "GUC parser failed ({}): {}",
        parsed.status,
        String::from_utf8_lossy(&parsed.stderr)
    );
    let records: Vec<Fields> =
        serde_json::from_slice(&parsed.stdout).expect("invalid JSON from the GUC parser");
    let mut entries = BTreeMap::new();
    for fields in records {
        let name = fields.get("name").expect("GUC missing name").clone();
        assert!(
            entries.insert(name.clone(), fields).is_none(),
            "duplicate GUC {name}"
        );
    }
    entries
}

pub fn build(source: &Path) {
    let entries = parameters(source);
    let options = super::guc_options::parse(source, &entries);
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let constants = constants(
        &fs::read_to_string(output.join("constants.rs"))
            .expect("cannot read generated PostgreSQL constants"),
    );
    let mut table = phf_codegen::Map::new();
    let mut skipped = String::new();
    let mut included = 0;
    let mut omitted = 0;
    for (name, fields) in &entries {
        match generate(fields, &constants, &options) {
            Ok(value) => {
                // phf_codegen accepts each value as Rust source text.
                table.entry(name.as_str(), value.to_string());
                included += 1;
            }
            Err(reason) => {
                skipped.push_str(&format!("{name}: {reason}\n"));
                omitted += 1;
            }
        }
    }
    let report = output.join("guc-skipped.txt");
    fs::write(&report, skipped).expect("cannot write skipped GUC report");
    if omitted > 0 {
        println!(
            "cargo:warning=Generated {included} GUCs; skipped {omitted}. See {}",
            report.display()
        );
    }
    let table: syn::Expr = syn::parse_str(&table.build().to_string())
        .expect("invalid Rust expression from phf_codegen");
    let generated: syn::File = syn::parse2(quote! {
        /// GUCs with resolved defaults and bounds, keyed by exact parameter name.
        pub static GUC: phf::Map<&'static str, Guc> = #table;
    })
    .expect("invalid generated GUC syntax");
    fs::write(output.join("guc.rs"), prettyplease::unparse(&generated))
        .expect("cannot write generated GUC table");
}

pub type Fields = BTreeMap<String, String>;

/// Remove C comments outside quoted strings without evaluating expressions.
pub fn without_comments(value: &str) -> String {
    let mut result = String::new();
    let mut chars = value.chars().peekable();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        if quoted {
            result.push(ch);
            match ch {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        result.push(escaped);
                    }
                }
                '"' => quoted = false,
                _ => {}
            }
        } else if ch == '"' {
            quoted = true;
            result.push(ch);
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            loop {
                let ch = chars.next().expect("unterminated C comment in GUC value");
                if ch == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result.trim().to_owned()
}

/// Keep Rust-compatible arithmetic intact and collect its constant references.
pub fn arithmetic(value: &str) -> Result<(syn::Expr, BTreeSet<String>), String> {
    fn collect(expr: &syn::Expr, names: &mut BTreeSet<String>) -> Result<(), String> {
        match expr {
            syn::Expr::Binary(binary)
                if matches!(
                    binary.op,
                    syn::BinOp::Add(_)
                        | syn::BinOp::Sub(_)
                        | syn::BinOp::Mul(_)
                        | syn::BinOp::Div(_)
                        | syn::BinOp::Rem(_)
                ) =>
            {
                collect(&binary.left, names)?;
                collect(&binary.right, names)
            }
            syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
                collect(&unary.expr, names)
            }
            syn::Expr::Paren(paren) => collect(&paren.expr, names),
            syn::Expr::Path(path) if path.qself.is_none() && path.path.get_ident().is_some() => {
                names.insert(path.path.get_ident().unwrap().to_string());
                Ok(())
            }
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(number),
                ..
            }) if number.suffix().is_empty() => {
                let token = number.to_string();
                if token.starts_with('0') && token.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
                {
                    return Err("C octal literals in arithmetic require conversion".to_owned());
                }
                Ok(())
            }
            _ => Err("unsupported arithmetic syntax".to_owned()),
        }
    }
    let expr = syn::parse_str(value).map_err(|_| format!("unsupported C expression {value:?}"))?;
    let mut names = BTreeSet::new();
    collect(&expr, &mut names)?;
    Ok((expr, names))
}

#[derive(Clone, Copy)]
enum ConstantType {
    Bool,
    Int,
    Real,
    String,
}

// Read only exported constants, never extern statics. Resolve bindgen's enum
// aliases so generation can validate types without evaluating their values.
fn constants(source: &str) -> BTreeMap<String, ConstantType> {
    let bindings = syn::parse_file(source).expect("invalid Rust from bindgen");
    let aliases: BTreeMap<_, _> = bindings
        .items
        .iter()
        .filter_map(|item| {
            if let syn::Item::Type(alias) = item {
                Some((alias.ident.to_string(), alias.ty.as_ref()))
            } else {
                None
            }
        })
        .collect();
    bindings
        .items
        .iter()
        .filter_map(|item| {
            let syn::Item::Const(item) = item else {
                return None;
            };
            if !matches!(item.vis, syn::Visibility::Public(_)) {
                return None;
            }
            Some((
                item.ident.to_string(),
                constant_type(&item.ty, &aliases, 0)?,
            ))
        })
        .collect()
}

fn constant_type(
    ty: &syn::Type,
    aliases: &BTreeMap<String, &syn::Type>,
    depth: usize,
) -> Option<ConstantType> {
    if depth > aliases.len() {
        return None;
    }
    match ty {
        syn::Type::Path(path) => {
            let name = path.path.segments.last()?.ident.to_string();
            if let Some(alias) = aliases.get(&name) {
                return constant_type(alias, aliases, depth + 1);
            }
            match name.as_str() {
                "bool" => Some(ConstantType::Bool),
                "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64"
                | "usize" | "c_char" | "c_schar" | "c_uchar" | "c_short" | "c_ushort" | "c_int"
                | "c_uint" | "c_long" | "c_ulong" | "c_longlong" | "c_ulonglong" => {
                    Some(ConstantType::Int)
                }
                "f32" | "f64" | "c_float" | "c_double" => Some(ConstantType::Real),
                _ => None,
            }
        }
        syn::Type::Reference(reference) if reference.mutability.is_none() => {
            let element = match reference.elem.as_ref() {
                syn::Type::Array(array) => &array.elem,
                syn::Type::Slice(slice) => &slice.elem,
                _ => return None,
            };
            matches!(element.as_ref(), syn::Type::Path(path) if path.path.is_ident("u8"))
                .then_some(ConstantType::String)
        }
        _ => None,
    }
}

fn resolve(
    kind: &str,
    value: &str,
    constants: &BTreeMap<String, ConstantType>,
) -> Result<TokenStream, String> {
    let cleaned = without_comments(value);
    let value = cleaned.as_str();
    if let Some(literal) = literal(kind, value) {
        return Ok(literal);
    }
    if matches!(kind, "int" | "enum") {
        let (expr, names) = arithmetic(value)?;
        let mut declarations = Vec::new();
        for name in names {
            if !matches!(constants.get(&name), Some(ConstantType::Int)) {
                return Err(format!("unresolved or non-integer constant {name}"));
            }
            let name = format_ident!("{name}");
            // Bindgen emits both signed and unsigned constants. Normalize operands
            // locally while leaving the source expression and its precedence intact.
            declarations.push(quote! {
                #[allow(non_upper_case_globals)]
                const #name: i128 = crate::constants::#name as i128;
            });
        }
        let variant = integer_variant(kind);
        let mut expr = &expr;
        while let syn::Expr::Paren(paren) = expr {
            expr = &paren.expr;
        }
        let expression = if declarations.is_empty() {
            quote!(#expr)
        } else {
            quote!({ #(#declarations)* #expr })
        };
        return Ok(quote!(Value::#variant(integer_constant(#expression))));
    }
    let Some(ty) = constants.get(value) else {
        return Err(format!(
            "unresolved constant or unsupported expression {value:?}"
        ));
    };
    let name = format_ident!("{value}");
    let reference = quote!(crate::constants::#name);
    match (kind, ty) {
        ("bool", ConstantType::Bool) => Ok(quote!(Value::Bool(#reference))),
        ("bool", ConstantType::Int) => Ok(quote!(Value::Bool(#reference != 0))),
        ("real", ConstantType::Int | ConstantType::Real) => {
            Ok(quote!(Value::Real(#reference as f64)))
        }
        ("string", ConstantType::String) => Ok(quote!(Value::String(string_constant(#reference)))),
        _ => Err(format!(
            "constant {value} has an incompatible type for {kind}"
        )),
    }
}

fn generate(
    fields: &Fields,
    constants: &BTreeMap<String, ConstantType>,
    options: &BTreeMap<String, Vec<String>>,
) -> Result<TokenStream, String> {
    if fields.contains_key("ifdef") {
        return Err(format!("conditional entry ({})", fields["ifdef"]));
    }
    let kind = fields.get("type").expect("GUC missing type");
    let resolve_field = |field: &str| {
        let value = fields
            .get(field)
            .unwrap_or_else(|| panic!("GUC missing {field}"));
        resolve(kind, value, constants).map_err(|reason| format!("{field}: {reason}"))
    };
    let boot = resolve_field("boot_val")?;
    let flags = without_comments(fields.get("flags").map(String::as_str).unwrap_or("0"));
    let report = flags.split('|').any(|flag| flag.trim() == "GUC_REPORT");
    // Only unit flags are passed to parse_int/parse_real.
    let units: Vec<_> = flags
        .split('|')
        .map(str::trim)
        .filter(|flag| flag.starts_with("GUC_UNIT_"))
        .map(|flag| {
            assert!(
                matches!(constants.get(flag), Some(ConstantType::Int)),
                "unknown GUC unit {flag}"
            );
            format_ident!("{flag}")
        })
        .collect();
    assert!(units.len() <= 1, "multiple GUC base units");
    let unit_flags = units
        .first()
        .map_or_else(|| quote!(0), |unit| quote!(crate::constants::#unit));
    let options = if kind == "enum" {
        options[&fields["options"]].as_slice()
    } else {
        &[]
    };
    let (min, max) = if matches!(kind.as_str(), "int" | "real") {
        let min = resolve_field("min")?;
        let max = resolve_field("max")?;
        (quote!(Some(#min)), quote!(Some(#max)))
    } else {
        (quote!(None), quote!(None))
    };
    let [name, context, group, short_desc] =
        ["name", "context", "group", "short_desc"].map(|key| {
            fields
                .get(key)
                .unwrap_or_else(|| panic!("GUC missing {key}"))
        });
    let long_desc = match fields.get("long_desc") {
        Some(description) => quote!(Some(#description)),
        None => quote!(None),
    };
    Ok(quote! {
        Guc {
            name: #name,
            context: #context,
            group: #group,
            short_desc: #short_desc,
            long_desc: #long_desc,
            boot_val: #boot,
            unit_flags: #unit_flags,
            report: #report,
            min: #min,
            max: #max,
            options: &[#(#options),*],
        }
    })
}

fn integer_variant(kind: &str) -> syn::Ident {
    if kind == "enum" {
        format_ident!("Enum")
    } else {
        format_ident!("Int")
    }
}

// Return Rust syntax only for literals; do not evaluate C constants or expressions.
fn literal(kind: &str, value: &str) -> Option<TokenStream> {
    match kind {
        "bool" => {
            let value = value.parse::<bool>().ok()?;
            Some(quote!(Value::Bool(#value)))
        }
        "int" | "enum" => {
            let unsigned = value.strip_prefix('-').unwrap_or(value);
            let (digits, radix) = if let Some(hex) = unsigned
                .strip_prefix("0x")
                .or_else(|| unsigned.strip_prefix("0X"))
            {
                (hex, 16)
            } else if unsigned.starts_with('0') && unsigned.len() > 1 {
                (unsigned, 8)
            } else {
                (unsigned, 10)
            };
            if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
                return None;
            }
            let number = i64::from_str_radix(digits, radix).ok()?;
            let number = if value.starts_with('-') {
                -number
            } else {
                number
            };
            let variant = integer_variant(kind);
            let number = i32::try_from(number).ok()?;
            Some(quote!(Value::#variant(#number)))
        }
        "real" => {
            let number = value.parse::<f64>().ok()?;
            number.is_finite().then(|| quote!(Value::Real(#number)))
        }
        "string" => {
            if value == "NULL" {
                return Some(quote!(Value::Null));
            }
            let decoded = c_string_literal(value)?;
            Some(quote!(Value::String(#decoded)))
        }
        _ => None,
    }
}

pub fn c_string_literal(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut decoded = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        decoded.push(match ch {
            '"' => return None,
            '\\' => match chars.next()? {
                '"' => '"',
                '\\' => '\\',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                _ => return None,
            },
            _ => ch,
        });
    }
    Some(decoded)
}
