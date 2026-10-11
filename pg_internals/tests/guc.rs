use pg_internals::guc::{GUC, Value};

#[test]
fn statement_timeout() {
    let guc = GUC
        .get("statement_timeout")
        .expect("missing statement_timeout");

    assert_eq!(guc.name, "statement_timeout");
    assert_eq!(guc.context, "PGC_USERSET");
    assert_eq!(guc.boot_val, Value::Int(0));
    assert_eq!(guc.min, Some(Value::Int(0)));
    assert_eq!(guc.max, Some(Value::Int(i32::MAX)));
    assert!(guc.options.is_empty());
}

#[test]
fn intervalstyle() {
    let guc = GUC.get("IntervalStyle").expect("missing intervalstyle");
    println!("{:?}", guc);
    assert_eq!(
        guc.options,
        &["postgres", "postgres_verbose", "sql_standard", "iso_8601"]
    );
}

#[test]
fn enum_options_from_external_tables_include_aliases() {
    let guc = GUC.get("archive_mode").expect("missing archive_mode");
    assert!(guc.options.contains(&"always"));
    assert!(guc.options.contains(&"on"));
    assert!(guc.options.contains(&"true"));
    assert!(guc.options.contains(&"off"));
}

#[test]
fn enum_table_offsets_exclude_leading_options() {
    let minimum = GUC.get("ssl_min_protocol_version").unwrap();
    let maximum = GUC.get("ssl_max_protocol_version").unwrap();
    assert_eq!(maximum.options.first(), Some(&""));
    assert_eq!(minimum.options, &maximum.options[1..]);
    assert!(!minimum.options.is_empty());
}
