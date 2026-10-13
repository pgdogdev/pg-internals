# Postgres internals

This crate gives Rust access to whatever Postgres internals we need to make [PgDog](https://github.com/pgdogdev/pgdog) work. It's compiled against a pinned commit from `master`, so only internals we can rely on are the ones that are backwards compatible. Thankfully, for Postgres, that's most of them.

## Features

### GUC

This crate provides access to GUC (Grand Unified Configuration) table for all settings. This gives us the ability to check if a setting we receive via `SET` is valid, e.g.,:

```sql
SET intervalstyle TO 'sdf';
```

can be checked against

```rust
crate::guc::GUC.get("IntervalStyle").unwrap().options;
```

We can also see if a user tries to configure a setting that Postgres doesn't have. Postgres doesn't remove settings (or renames them since Postgres 10), so this will pretty much always work.
