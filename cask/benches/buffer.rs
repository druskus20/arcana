use criterion::{Criterion, criterion_group, criterion_main};

use cask::Buffer;

fn bench_copy_from_slice(c: &mut Criterion) {
    let data: Vec<u32> = (0..10_000).collect();
    let mut buffer = Buffer::<u32>::with_capacity(10_000);
    c.bench_function("copy_from_slice", |b| {
        b.iter(|| {
            buffer.copy_from_slice(&data);
        })
    });
}

fn bench_copy_at_most_n(c: &mut Criterion) {
    let data: Vec<u32> = (0..10_000).collect();
    let mut buffer = Buffer::<u32>::with_capacity(10_000);
    c.bench_function("copy_from_iter", |b| {
        b.iter(|| {
            buffer.copy_at_most_n(data.iter().cloned(), 10_000);
        })
    });
}

criterion_group!(benches, bench_copy_from_slice, bench_copy_at_most_n);
criterion_main!(benches);
