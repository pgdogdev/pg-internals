//! Threaded workload for Valgrind: memory-check [threads] [iterations per thread].

use std::{env, error::Error, num::NonZeroUsize, sync::Barrier, thread};

use pg_internals::{
    constants::{
        BLCKSZ, GUC_UNIT_BLOCKS, GUC_UNIT_BYTE, GUC_UNIT_KB, GUC_UNIT_MB, GUC_UNIT_MIN,
        GUC_UNIT_MS, GUC_UNIT_S, GUC_UNIT_XBLOCKS, XLOG_BLCKSZ,
    },
    guc::{DateStyleError, DateStyleInput, GUC, Value, normalize_datestyle, parse_int, parse_real},
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let threads = args
        .next()
        .unwrap_or_else(|| "8".into())
        .parse::<NonZeroUsize>()?
        .get();
    let iterations = args
        .next()
        .unwrap_or_else(|| "1000".into())
        .parse::<NonZeroUsize>()?
        .get();
    if args.next().is_some() {
        return Err("usage: memory-check [threads] [iterations per thread]".into());
    }

    let start = Barrier::new(threads);
    thread::scope(|scope| {
        for worker in 0..threads {
            let start = &start;
            scope.spawn(move || {
                let settings = ["ISO, MDY", "SQL, YMD", "German, DMY", "Postgres, MDY"];
                let current = settings[worker % settings.len()];
                let reset = settings[(worker + 1) % settings.len()];
                let long_list = vec!["ISO"; 128].join(", ");
                let long_keyword = "é".repeat(128);
                start.wait();
                for iteration in 0..iterations {
                    check_numbers(worker, iteration);
                    check_datestyle(current, reset, &long_list, &long_keyword);
                }
            });
        }
    }); // Join all workers and drop their allocations before Valgrind checks leaks.
    println!("Memory check completed: {threads} threads, {iterations} iterations each");
    Ok(())
}

fn check_numbers(worker: usize, iteration: usize) {
    let number = ((worker % 100) * 100 + iteration % 100) as i32;
    let text = number.to_string();
    assert_eq!(parse_int(&text, 0), Some(number));
    assert_eq!(parse_real(&text, 0), Some(f64::from(number)));

    for (input, flags, expected) in [
        ("010", 0, 8),
        ("0x10", 0, 16),
        ("-1.5", 0, -2),
        ("2147483647", 0, i32::MAX),
        ("-2147483648", 0, i32::MIN),
        ("16MB", GUC_UNIT_BYTE, 16 * 1024 * 1024),
        ("16MB", GUC_UNIT_KB, 16 * 1024),
        ("16MB", GUC_UNIT_MB, 16),
        ("16MB", GUC_UNIT_BLOCKS, 16 * 1024 * 1024 / BLCKSZ),
        ("16MB", GUC_UNIT_XBLOCKS, 16 * 1024 * 1024 / XLOG_BLCKSZ),
        (" 1.5 min ", GUC_UNIT_MS, 90_000),
        ("1.5 min", GUC_UNIT_S, 90),
        ("2h", GUC_UNIT_MIN, 120),
    ] {
        assert_eq!(parse_int(input, flags), Some(expected));
    }
    assert_eq!(parse_real("1.25", 0), Some(1.25));
    assert_eq!(parse_real("0.5s", GUC_UNIT_MS), Some(500.0));
    assert_eq!(parse_real("Infinity", 0), Some(f64::INFINITY));
    for input in ["", "NaN", "1e309", "1e-9999", "1\0", "junk"] {
        assert_eq!(parse_int(input, 0), None);
        assert_eq!(parse_real(input, 0), None);
    }
    for input in ["2147483648", "-2147483649", "inf"] {
        assert_eq!(parse_int(input, 0), None);
    }
    for input in ["5MB", "5MS", "5ms junk"] {
        assert_eq!(parse_int(input, GUC_UNIT_MS), None);
        assert_eq!(parse_real(input, GUC_UNIT_MS), None);
    }

    // Exercise every setting, including all numeric units and nonnumeric types.
    for guc in GUC.values() {
        let expected = match guc.boot_val {
            Value::Int(_) => Some(Value::Int(number)),
            Value::Real(_) => Some(Value::Real(f64::from(number))),
            _ => None,
        };
        assert_eq!(guc.parse_numeric(&text), expected, "{}", guc.name);
        assert_eq!(guc.parse_numeric("invalid"), None, "{}", guc.name);
    }
}

fn check_datestyle(current: &str, reset: &str, long_list: &str, long_keyword: &str) {
    assert_eq!(normalize_datestyle("", current, reset).unwrap(), current);
    assert_eq!(
        normalize_datestyle("DEFAULT", current, reset).unwrap(),
        reset
    );
    for (input, expected) in [
        ("postgresql, european", "Postgres, DMY"),
        ("\"SQL\", \"YMD\"", "SQL, YMD"),
        ("German, MDY", "German, MDY"),
        (long_list, "ISO, YMD"),
    ] {
        assert_eq!(
            normalize_datestyle(input, "SQL, YMD", reset).unwrap(),
            expected
        );
    }
    for input in [
        "ISO, SQL",
        "MDY, YMD",
        "ISO,",
        "\"ISO",
        "\"A\"\"B\"",
        long_keyword,
    ] {
        let error = normalize_datestyle(input, current, reset).unwrap_err();
        assert!(matches!(
            error,
            DateStyleError::Invalid {
                input: DateStyleInput::Value,
                ..
            }
        ));
        assert!(!error.to_string().is_empty());
    }
    for (value, current, reset, input) in [
        ("ISO\0", current, reset, DateStyleInput::Value),
        ("ISO", "ISO\0", reset, DateStyleInput::Current),
        ("ISO", current, "ISO\0", DateStyleInput::Reset),
    ] {
        let error = normalize_datestyle(value, current, reset).unwrap_err();
        assert_eq!(error, DateStyleError::InteriorNul { input });
        assert!(!error.to_string().is_empty());
    }
    for (current, reset, input) in [
        ("invalid", reset, DateStyleInput::Current),
        (current, "invalid", DateStyleInput::Reset),
    ] {
        let error = normalize_datestyle("ISO", current, reset).unwrap_err();
        assert!(matches!(error, DateStyleError::Invalid { input: actual, .. } if actual == input));
        assert!(!error.to_string().is_empty());
    }
}
