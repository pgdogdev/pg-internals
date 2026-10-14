use pg_internals::{
    constants::{BLCKSZ, GUC_UNIT_BLOCKS, GUC_UNIT_KB, GUC_UNIT_MS, GUC_UNIT_XBLOCKS, XLOG_BLCKSZ},
    guc::{GUC, Value, parse_int, parse_real},
};

#[test]
fn units_follow_each_setting() {
    for (name, input, expected) in [
        ("statement_timeout", "5s", 5000),
        ("statement_timeout", "1.5 min", 90000),
        ("statement_timeout", "12345ms", 12345),
        ("work_mem", "16MB", 16384),
        ("shared_buffers", "16MB", 16 * 1024 * 1024 / BLCKSZ),
        ("extra_float_digits", "2", 2),
    ] {
        let guc = GUC.get(name).expect("known setting");
        assert_eq!(
            guc.parse_numeric(input),
            Some(Value::Int(expected)),
            "{name}: {input}"
        );
    }
    assert_eq!(GUC["statement_timeout"].unit_flags, GUC_UNIT_MS);
    assert_eq!(GUC["work_mem"].unit_flags, GUC_UNIT_KB);
    assert_eq!(GUC["shared_buffers"].unit_flags, GUC_UNIT_BLOCKS);
    assert_eq!(GUC["extra_float_digits"].unit_flags, 0);
    assert_eq!(
        parse_int("1MB", GUC_UNIT_XBLOCKS),
        Some(1024 * 1024 / XLOG_BLCKSZ)
    );
    assert_eq!(
        GUC["vacuum_cost_delay"].parse_numeric("0.5s"),
        Some(Value::Real(500.0))
    );
    assert_eq!(GUC["application_name"].parse_numeric("1"), None);
}

#[test]
fn integer_syntax_and_rounding_match_postgres() {
    for (input, expected) in [
        ("  +42  ", 42),
        ("010", 8),
        ("0x10", 16),
        ("1e2", 100),
        ("1.5", 2),
        ("2.5", 2),
        ("-1.5", -2),
        ("-2.5", -2),
        ("2147483647", i32::MAX),
        ("-2147483648", i32::MIN),
    ] {
        assert_eq!(parse_int(input, 0), Some(expected), "{input}");
    }
    assert_eq!(parse_int("0.0009s", GUC_UNIT_MS), Some(1));
    assert_eq!(parse_real("0.0009s", GUC_UNIT_MS), Some(1.0));
    assert_eq!(parse_int("1.0005MB", GUC_UNIT_KB), Some(1025));
    assert_eq!(parse_int(" 5 ms \t", GUC_UNIT_MS), Some(5));
}

#[test]
fn invalid_numbers_units_and_overflow() {
    for input in [
        "",
        " ",
        "NaN",
        "inf",
        "-inf",
        "1e309",
        "1e-9999",
        "08",
        "1\0ms",
        "2147483648",
        "-2147483649",
    ] {
        assert_eq!(parse_int(input, 0), None, "{input:?}");
    }
    for input in [
        "5MB",
        "5MS",
        "5m",
        "5ms junk",
        "5m s",
        "2147483648ms",
        "99999999999999999999d",
    ] {
        assert_eq!(parse_int(input, GUC_UNIT_MS), None, "{input}");
    }
    assert_eq!(parse_int("5s", 0), None);
    assert_eq!(parse_int("5s", GUC_UNIT_KB), None);
    assert_eq!(parse_real("5MB", GUC_UNIT_MS), None);
    assert_eq!(parse_real("5s", 0), None);
    for input in ["", "NaN", "1e309", "1e-9999", "1\0"] {
        assert_eq!(parse_real(input, 0), None, "{input:?}");
    }
}

#[test]
fn real_parsing_and_setting_bounds_are_separate() {
    assert_eq!(parse_real("1.25", 0), Some(1.25));
    assert_eq!(parse_real("Infinity", 0), Some(f64::INFINITY));
    let guc = GUC.get("extra_float_digits").expect("known setting");
    assert_eq!(guc.max, Some(Value::Int(3)));
    assert_eq!(guc.parse_numeric("4"), Some(Value::Int(4)));
}
