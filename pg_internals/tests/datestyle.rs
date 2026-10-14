use pg_internals::guc::{DateStyleError, DateStyleInput, normalize_datestyle};
use std::thread;

#[test]
fn partial_assignments_preserve_the_other_component() {
    for (value, current, expected) in [
        ("MDY", "ISO, YMD", "ISO, MDY"),
        ("SQL", "ISO, YMD", "SQL, YMD"),
        ("YMD", "Postgres, DMY", "Postgres, YMD"),
        ("German", "ISO, MDY", "German, DMY"),
        ("German, MDY", "ISO, YMD", "German, MDY"),
        ("MDY, German", "ISO, YMD", "German, MDY"),
        ("ISO, ISO, DMY, DMY", "SQL, YMD", "ISO, DMY"),
        ("", "SQL, YMD", "SQL, YMD"),
        (" \t\n", "ISO, MDY", "ISO, MDY"),
    ] {
        assert_eq!(
            normalize_datestyle(value, current, "ISO, MDY").expect("valid setting"),
            expected,
            "{value:?}"
        );
    }
}

#[test]
fn defaults_use_the_supplied_reset_setting() {
    for (value, expected) in [
        ("DEFAULT", "SQL, YMD"),
        ("DEFAULT, ISO", "ISO, YMD"),
        ("ISO, DEFAULT", "ISO, YMD"),
        ("DEFAULT, MDY", "SQL, MDY"),
        ("MDY, DEFAULT", "SQL, MDY"),
        ("German, DEFAULT", "German, YMD"),
        ("DEFAULT, German", "German, DMY"),
        ("DEFAULT, DEFAULT", "SQL, YMD"),
    ] {
        assert_eq!(
            normalize_datestyle(value, "German, DMY", "SQL, YMD").expect("valid default"),
            expected
        );
    }
    assert_eq!(
        normalize_datestyle("DEFAULT", "DEFAULT", "DEFAULT").expect("built-in default"),
        "ISO, MDY"
    );
    assert_eq!(
        normalize_datestyle("", "MDY", "German").expect("partial context"),
        "German, MDY"
    );
}

#[test]
fn aliases_case_and_quoted_identifiers_follow_postgres() {
    for (value, expected) in [
        ("postgresql, european", "Postgres, DMY"),
        ("sql, noneuro", "SQL, MDY"),
        ("iso, us", "ISO, MDY"),
        ("POSTGRESanything, EUROanything", "Postgres, DMY"),
        ("SQL, NONEUROanything", "SQL, MDY"),
        ("\"SQL\", \"YMD\"", "SQL, YMD"),
        (" \tIso\n,\rDmy\x0b", "ISO, DMY"),
    ] {
        assert_eq!(
            normalize_datestyle(value, "ISO, YMD", "ISO, MDY").expect("valid alias"),
            expected
        );
    }
}

#[test]
fn invalid_settings_preserve_postgres_error_details() {
    for (value, detail) in [
        ("ISO, SQL", "Conflicting \"DateStyle\" specifications."),
        ("MDY, YMD", "Conflicting \"DateStyle\" specifications."),
        ("asdf", "Unrecognized key word: \"asdf\"."),
        ("\"A\"\"B\"", "Unrecognized key word: \"A\"B\"."),
        ("ISO, SQL, ASDF", "Unrecognized key word: \"asdf\"."),
        ("ASDF,", "List syntax is invalid."),
        ("\"ISO\"x", "List syntax is invalid."),
        ("\"\"", "Unrecognized key word: \"\"."),
        ("\u{a0}", "Unrecognized key word: \"\u{a0}\"."),
        ("ISO,", "List syntax is invalid."),
        (",ISO", "List syntax is invalid."),
        ("ISO DMY", "List syntax is invalid."),
        ("\"ISO", "List syntax is invalid."),
    ] {
        assert_eq!(
            normalize_datestyle(value, "ISO, MDY", "ISO, MDY"),
            Err(DateStyleError::Invalid {
                input: DateStyleInput::Value,
                detail: detail.into()
            }),
            "{value:?}"
        );
    }
    let long = "é".repeat(40);
    assert_eq!(
        normalize_datestyle(&long, "ISO, MDY", "ISO, MDY"),
        Err(DateStyleError::Invalid {
            input: DateStyleInput::Value,
            detail: format!("Unrecognized key word: \"{}\".", "é".repeat(31))
        })
    );
}

#[test]
fn context_errors_and_nuls_identify_the_argument() {
    for (value, current, reset, input) in [
        ("ISO\0", "ISO, MDY", "ISO, MDY", DateStyleInput::Value),
        ("ISO", "ISO\0", "ISO, MDY", DateStyleInput::Current),
        ("ISO", "ISO, MDY", "ISO\0", DateStyleInput::Reset),
    ] {
        assert_eq!(
            normalize_datestyle(value, current, reset),
            Err(DateStyleError::InteriorNul { input })
        );
    }
    for (current, reset, input) in [
        ("invalid", "ISO, MDY", DateStyleInput::Current),
        ("ISO, MDY", "invalid", DateStyleInput::Reset),
    ] {
        assert_eq!(
            normalize_datestyle("ISO", current, reset),
            Err(DateStyleError::Invalid {
                input,
                detail: "Unrecognized key word: \"invalid\".".into()
            })
        );
    }
}

#[test]
fn long_lists_and_repeated_errors_do_not_leak_state() {
    let repeated = vec!["ISO"; 512].join(", ");
    for _ in 0..100 {
        assert!(normalize_datestyle("SQL, ISO", "German, DMY", "SQL, YMD").is_err());
        assert_eq!(
            normalize_datestyle(&repeated, "SQL, YMD", "SQL, DMY").expect("long list"),
            "ISO, YMD"
        );
    }
}

#[test]
fn concurrent_calls_keep_client_contexts_separate() {
    let handles: Vec<_> = ["ISO, MDY", "SQL, YMD", "German, DMY", "Postgres, MDY"]
        .into_iter()
        .map(|setting| {
            thread::spawn(move || {
                for _ in 0..500 {
                    assert_eq!(
                        normalize_datestyle("DEFAULT", "ISO, MDY", setting)
                            .expect("independent default"),
                        setting
                    );
                    assert_eq!(
                        normalize_datestyle("", setting, "SQL, DMY").expect("independent current"),
                        setting
                    );
                    assert!(normalize_datestyle("invalid", setting, "ISO, MDY").is_err());
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("normalization thread completed");
    }
}
