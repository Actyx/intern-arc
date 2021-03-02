/*
 * Copyright 2021 Actyx AG
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use intern_arc::{InternHash, InternOrd};
use quickcheck::{Arbitrary, Gen};

fn large(size: usize) -> Vec<u8> {
    Arbitrary::arbitrary(&mut Gen::new(size))
}

fn hash(c: &mut Criterion) {
    let mut c = c.benchmark_group("hash");

    c.bench_function("intern fresh", |b| {
        b.iter_batched_ref(
            InternHash::<str>::new,
            |i| i.intern_ref("hello"),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern known", |b| {
        b.iter_batched_ref(
            || {
                let i = InternHash::<str>::new();
                i.intern_ref("hello");
                i
            },
            |i| i.intern_ref("hello"),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern & drop fresh", |b| {
        b.iter_batched_ref(
            InternHash::<str>::new,
            |i| {
                i.intern_ref("hello");
            },
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern & drop known", |b| {
        b.iter_batched_ref(
            || {
                let i = InternHash::<str>::new();
                i.intern_ref("hello");
                i
            },
            |i| {
                i.intern_ref("hello");
            },
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern when crowded", |b| {
        b.iter_batched_ref(
            || {
                let i = InternHash::<u32>::new();
                for n in 0..100_000 {
                    std::mem::forget(i.intern_sized(n));
                }
                i
            },
            |i| i.intern_sized(12345678),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern large values", |b| {
        b.iter_batched_ref(
            || {
                let i = InternHash::<[u8]>::new();
                for _ in 0..100 {
                    std::mem::forget(i.intern_box(large(500_000).into()))
                }
                i
            },
            |i| i.intern_box(large(500_000).into()),
            BatchSize::LargeInput,
        )
    });
    c.bench_function("intern large existing values", |b| {
        b.iter_batched_ref(
            || {
                let i = InternHash::<[u8]>::new();
                for _ in 0..100 {
                    std::mem::forget(i.intern_box(large(500_000).into()))
                }
                let v = i.intern_box(large(500_000).into());
                (i, v)
            },
            |(i, v)| i.intern_ref(v.as_ref()),
            BatchSize::LargeInput,
        )
    });
    c.bench_function("intern medium existing values", |b| {
        b.iter_batched_ref(
            || {
                let i = InternHash::<[u8]>::new();
                for _ in 0..1000 {
                    std::mem::forget(i.intern_box(large(50_000).into()))
                }
                let v = i.intern_box(large(50_000).into());
                (i, v)
            },
            |(i, v)| i.intern_ref(v.as_ref()),
            BatchSize::LargeInput,
        )
    });
    c.finish();
}

fn ord(c: &mut Criterion) {
    let mut c = c.benchmark_group("ord");

    c.bench_function("intern fresh", |b| {
        b.iter_batched_ref(
            InternOrd::<str>::new,
            |i| i.intern_ref("hello"),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern known", |b| {
        b.iter_batched_ref(
            || {
                let i = InternOrd::<str>::new();
                i.intern_ref("hello");
                i
            },
            |i| i.intern_ref("hello"),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern & drop fresh", |b| {
        b.iter_batched_ref(
            InternOrd::<str>::new,
            |i| {
                i.intern_ref("hello");
            },
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern & drop known", |b| {
        b.iter_batched_ref(
            || {
                let i = InternOrd::<str>::new();
                i.intern_ref("hello");
                i
            },
            |i| {
                i.intern_ref("hello");
            },
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern when crowded", |b| {
        b.iter_batched_ref(
            || {
                let i = InternOrd::<u32>::new();
                for n in 0..100_000 {
                    std::mem::forget(i.intern_sized(n));
                }
                i
            },
            |i| i.intern_sized(12345678),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern when moderately crowded", |b| {
        b.iter_batched_ref(
            || {
                let i = InternOrd::<u32>::new();
                for n in 0..100 {
                    std::mem::forget(i.intern_sized(n));
                }
                i
            },
            |i| i.intern_sized(12345678),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("intern large values", |b| {
        b.iter_batched_ref(
            || {
                let i = InternOrd::<[u8]>::new();
                for _ in 0..100 {
                    std::mem::forget(i.intern_box(large(500_000).into()))
                }
                i
            },
            |i| i.intern_box(large(500_000).into()),
            BatchSize::LargeInput,
        )
    });
    c.bench_function("intern large existing values", |b| {
        b.iter_batched_ref(
            || {
                let i = InternOrd::<[u8]>::new();
                for _ in 0..100 {
                    std::mem::forget(i.intern_box(large(500_000).into()))
                }
                let v = i.intern_box(large(500_000).into());
                (i, v)
            },
            |(i, v)| i.intern_ref(v.as_ref()),
            BatchSize::LargeInput,
        )
    });
    c.bench_function("intern medium existing values", |b| {
        b.iter_batched_ref(
            || {
                let i = InternOrd::<[u8]>::new();
                for _ in 0..1000 {
                    std::mem::forget(i.intern_box(large(50_000).into()))
                }
                let v = i.intern_box(large(50_000).into());
                (i, v)
            },
            |(i, v)| i.intern_ref(v.as_ref()),
            BatchSize::LargeInput,
        )
    });
    c.finish();
}

criterion_group!(benches, hash, ord);
criterion_main!(benches);
