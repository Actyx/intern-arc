use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use intern_arc::{Intern, Interned};

fn intern_hello(interner: &Intern<str>) -> Interned<str> {
    interner.intern_ref("hello")
}

fn intern_and_drop(interner: &Intern<str>) {
    interner.intern_ref("hello");
}

fn hash(c: &mut Criterion) {
    let mut c = c.benchmark_group("hash");
    c.bench_function("intern fresh", |b| {
        b.iter_batched_ref(
            Intern::<str>::new,
            |i| intern_hello(i),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern known", |b| {
        b.iter_batched_ref(
            || {
                let i = Intern::<str>::new();
                i.intern_ref("hello");
                i
            },
            |i| intern_hello(i),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern & drop fresh", |b| {
        b.iter_batched_ref(
            Intern::<str>::new,
            |i| intern_and_drop(i),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern & drop known", |b| {
        b.iter_batched_ref(
            || {
                let i = Intern::<str>::new();
                i.intern_ref("hello");
                i
            },
            |i| intern_and_drop(i),
            BatchSize::SmallInput,
        )
    });
    c.finish();
}

fn ord(c: &mut Criterion) {
    let mut c = c.benchmark_group("ord");
    c.bench_function("intern fresh", |b| {
        b.iter_batched_ref(
            || Intern::<str>::with_hash_limit(0),
            |i| intern_hello(i),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern known", |b| {
        b.iter_batched_ref(
            || {
                let i = Intern::<str>::with_hash_limit(0);
                i.intern_ref("hello");
                i
            },
            |i| intern_hello(i),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern & drop fresh", |b| {
        b.iter_batched_ref(
            || Intern::<str>::with_hash_limit(0),
            |i| intern_and_drop(i),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern & drop known", |b| {
        b.iter_batched_ref(
            || {
                let i = Intern::<str>::with_hash_limit(0);
                i.intern_ref("hello");
                i
            },
            |i| intern_and_drop(i),
            BatchSize::SmallInput,
        )
    });
    c.finish();
}

criterion_group!(benches, hash, ord);
criterion_main!(benches);
