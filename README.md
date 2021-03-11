# Interning library based on atomic reference counting

For the docs see [docs.rs](https://docs.rs/intern_arc).

When developing please make use of `cargo bench` and `cargo +nightly miri` — the multithreading tests are sized such that they bring issues to the surface when run with miri (which is rather slow).
Once you notice problems, debugging is easier with

```bash
export MIRIFLAGS=-Zmiri-disable-isolation
cargo +nightly miri test --features println -- --nocapture multithreading_hash
```

Another very useful tool is [loom](https://docs.rs/loom), run it with

```bash
LOOM_LOG=1 LOOM_LOCATION=1 LOOM_CHECKPOINT_INTERVAL=1 RUST_BACKTRACE=1 RUSTFLAGS="--cfg loom" cargo test --lib
```

**Note:** `loom` currently does not work because it doesn’t support Weak references.

## License

At your option: Apache-2.0 or MIT
