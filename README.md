# Interning library based on atomic reference counting

For the docs see [docs.rs](https://docs.rs/intern_arc).

When developing please make use of `cargo bench` and `cargo +nightly miri` — the multithreading tests are sized such that they bring issues to the surface when run with miri (which is rather slow).
Once you notice problems, debugging is easier with

```bash
cargo +nightly miri test --features println -- --nocapture multithreading_hash
```

## License

At your option: Apache-2.0 or MIT
