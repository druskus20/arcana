 use criterion::{black_box, Criterion, criterion_group, criterion_main, BenchmarkId};
use cask::Buffer;
use std::collections::VecDeque;

// Test data sizes
const SMALL_SIZE: usize = 100;
const MEDIUM_SIZE: usize = 1_000;
const LARGE_SIZE: usize = 10_000;

// =============================================================================
// Copy Operations
// =============================================================================

fn bench_copy_from_slice_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("copy_from_slice");
    
    for &size in &[SMALL_SIZE, MEDIUM_SIZE, LARGE_SIZE] {
        let data: Vec<u32> = (0..size as u32).collect();
        
        // Buffer implementation
        group.bench_with_input(BenchmarkId::new("Buffer", size), &size, |b, &size| {
            let mut buffer = Buffer::<u32>::with_capacity(size);
            b.iter(|| {
                buffer.copy_from_slice(black_box(&data));
            })
        });
        
        // Vec implementation
        group.bench_with_input(BenchmarkId::new("Vec", size), &size, |b, &_size| {
            let mut vec = Vec::with_capacity(size);
            b.iter(|| {
                vec.clear();
                vec.extend_from_slice(black_box(&data));
            })
        });
        
        // VecDeque implementation
        group.bench_with_input(BenchmarkId::new("VecDeque", size), &size, |b, &_size| {
            let mut deque = VecDeque::with_capacity(size);
            b.iter(|| {
                deque.clear();
                deque.extend(black_box(data.iter().copied()));
            })
        });
    }
    
    group.finish();
}

fn bench_copy_at_most_n_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("copy_at_most_n");
    
    for &size in &[SMALL_SIZE, MEDIUM_SIZE, LARGE_SIZE] {
        let data: Vec<u32> = (0..size as u32).collect();
        
        // Buffer implementation
        group.bench_with_input(BenchmarkId::new("Buffer", size), &size, |b, &size| {
            let mut buffer = Buffer::<u32>::with_capacity(size);
            b.iter(|| {
                buffer.copy_at_most_n(black_box(data.iter().copied()), size);
            })
        });
        
        // Vec implementation (equivalent)
        group.bench_with_input(BenchmarkId::new("Vec", size), &size, |b, &size| {
            let mut vec = Vec::with_capacity(size);
            b.iter(|| {
                vec.clear();
                vec.extend(black_box(data.iter().copied()).take(size));
            })
        });
    }
    
    group.finish();
}

// =============================================================================
// Sequential Operations
// =============================================================================

fn bench_sequential_push_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("sequential_push");
    
    for &size in &[SMALL_SIZE, MEDIUM_SIZE, LARGE_SIZE] {
        // Buffer implementation
        group.bench_with_input(BenchmarkId::new("Buffer", size), &size, |b, &size| {
            b.iter(|| {
                let mut buffer = Buffer::<u32>::with_capacity(size);
                for i in 0..size as u32 {
                    buffer.push(black_box(i));
                }
                black_box(buffer);
            })
        });
        
        // Vec implementation
        group.bench_with_input(BenchmarkId::new("Vec", size), &size, |b, &size| {
            b.iter(|| {
                let mut vec = Vec::with_capacity(size);
                for i in 0..size as u32 {
                    vec.push(black_box(i));
                }
                black_box(vec);
            })
        });
        
        // VecDeque implementation
        group.bench_with_input(BenchmarkId::new("VecDeque", size), &size, |b, &size| {
            b.iter(|| {
                let mut deque = VecDeque::with_capacity(size);
                for i in 0..size as u32 {
                    deque.push_back(black_box(i));
                }
                black_box(deque);
            })
        });
    }
    
    group.finish();
}

fn bench_sequential_pop_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("sequential_pop");
    
    for &size in &[SMALL_SIZE, MEDIUM_SIZE, LARGE_SIZE] {
        // Buffer implementation
        group.bench_with_input(BenchmarkId::new("Buffer", size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let mut buffer = Buffer::<u32>::with_capacity(size);
                    for i in 0..size as u32 {
                        buffer.push(i);
                    }
                    buffer
                },
                |mut buffer| {
                    while let Some(_) = buffer.pop() {
                        // Pop all elements
                    }
                    black_box(buffer);
                },
                criterion::BatchSize::SmallInput
            )
        });
        
        // Vec implementation
        group.bench_with_input(BenchmarkId::new("Vec", size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let mut vec = Vec::with_capacity(size);
                    for i in 0..size as u32 {
                        vec.push(i);
                    }
                    vec
                },
                |mut vec| {
                    while let Some(_) = vec.pop() {
                        // Pop all elements
                    }
                    black_box(vec);
                },
                criterion::BatchSize::SmallInput
            )
        });
        
        // VecDeque implementation
        group.bench_with_input(BenchmarkId::new("VecDeque", size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let mut deque = VecDeque::with_capacity(size);
                    for i in 0..size as u32 {
                        deque.push_back(i);
                    }
                    deque
                },
                |mut deque| {
                    while let Some(_) = deque.pop_back() {
                        // Pop all elements
                    }
                    black_box(deque);
                },
                criterion::BatchSize::SmallInput
            )
        });
    }
    
    group.finish();
}

// =============================================================================
// Random Access Operations
// =============================================================================

fn bench_random_access_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("random_access");
    
    for &size in &[SMALL_SIZE, MEDIUM_SIZE, LARGE_SIZE] {
        let indices: Vec<usize> = (0..1000).map(|i| i % size).collect();
        
        // Buffer implementation
        group.bench_with_input(BenchmarkId::new("Buffer", size), &size, |b, &size| {
            let mut buffer = Buffer::<u32>::with_capacity(size);
            for i in 0..size as u32 {
                buffer.push(i);
            }
            
            b.iter(|| {
                let data = buffer.data();
                let mut sum = 0u32;
                for &idx in black_box(&indices) {
                    sum += data[idx];
                }
                black_box(sum);
            })
        });
        
        // Vec implementation
        group.bench_with_input(BenchmarkId::new("Vec", size), &size, |b, &_size| {
            let vec: Vec<u32> = (0..size as u32).collect();
            
            b.iter(|| {
                let mut sum = 0u32;
                for &idx in black_box(&indices) {
                    sum += vec[idx];
                }
                black_box(sum);
            })
        });
    }
    
    group.finish();
}

// =============================================================================
// Resize Operations
// =============================================================================

fn bench_resize_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_operations");
    
    for &initial_size in &[SMALL_SIZE, MEDIUM_SIZE] {
        let new_size = initial_size * 2;
        
        // Buffer implementation
        group.bench_with_input(
            BenchmarkId::new("Buffer", initial_size), 
            &initial_size, 
            |b, &initial_size| {
                b.iter_batched(
                    || {
                        let mut buffer = Buffer::<u32>::with_capacity(new_size);
                        for i in 0..initial_size as u32 {
                            buffer.push(i);
                        }
                        buffer
                    },
                    |mut buffer| {
                        buffer.resize_data(black_box(new_size));
                        black_box(buffer);
                    },
                    criterion::BatchSize::SmallInput
                )
            }
        );
        
        // Vec implementation
        group.bench_with_input(
            BenchmarkId::new("Vec", initial_size), 
            &initial_size, 
            |b, &initial_size| {
                b.iter_batched(
                    || {
                        let mut vec = Vec::with_capacity(new_size);
                        for i in 0..initial_size as u32 {
                            vec.push(i);
                        }
                        vec
                    },
                    |mut vec| {
                        vec.resize(black_box(new_size), 0u32);
                        black_box(vec);
                    },
                    criterion::BatchSize::SmallInput
                )
            }
        );
    }
    
    group.finish();
}

// =============================================================================
// Clear Operations
// =============================================================================

fn bench_clear_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("clear_operations");
    
    for &size in &[SMALL_SIZE, MEDIUM_SIZE, LARGE_SIZE] {
        // Buffer implementation
        group.bench_with_input(BenchmarkId::new("Buffer", size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let mut buffer = Buffer::<u32>::with_capacity(size);
                    for i in 0..size as u32 {
                        buffer.push(i);
                    }
                    buffer
                },
                |mut buffer| {
                    buffer.clear();
                    black_box(buffer);
                },
                criterion::BatchSize::SmallInput
            )
        });
        
        // Vec implementation
        group.bench_with_input(BenchmarkId::new("Vec", size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let vec: Vec<u32> = (0..size as u32).collect();
                    vec
                },
                |mut vec| {
                    vec.clear();
                    black_box(vec);
                },
                criterion::BatchSize::SmallInput
            )
        });
    }
    
    group.finish();
}

// =============================================================================
// Mixed Workload
// =============================================================================

fn bench_mixed_workload_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_workload");
    
    for &size in &[SMALL_SIZE, MEDIUM_SIZE] {
        // Buffer implementation
        group.bench_with_input(BenchmarkId::new("Buffer", size), &size, |b, &size| {
            b.iter(|| {
                let mut buffer = Buffer::<u32>::with_capacity(size);
                
                // Fill half
                for i in 0..(size / 2) as u32 {
                    buffer.push(i);
                }
                
                // Pop quarter
                for _ in 0..(size / 4) {
                    buffer.pop();
                }
                
                // Fill to capacity
                while !buffer.is_full() {
                    buffer.push(999);
                }
                
                // Access some elements
                let data = buffer.data();
                let mut sum = 0u32;
                for i in (0..data.len()).step_by(4) {
                    sum += data[i];
                }
                
                // Clear
                buffer.clear();
                
                black_box((buffer, sum));
            })
        });
        
        // Vec implementation
        group.bench_with_input(BenchmarkId::new("Vec", size), &size, |b, &size| {
            b.iter(|| {
                let mut vec = Vec::with_capacity(size);
                
                // Fill half
                for i in 0..(size / 2) as u32 {
                    vec.push(i);
                }
                
                // Pop quarter
                for _ in 0..(size / 4) {
                    vec.pop();
                }
                
                // Fill to capacity
                while vec.len() < size {
                    vec.push(999);
                }
                
                // Access some elements
                let mut sum = 0u32;
                for i in (0..vec.len()).step_by(4) {
                    sum += vec[i];
                }
                
                // Clear
                vec.clear();
                
                black_box((vec, sum));
            })
        });
    }
    
    group.finish();
}

// =============================================================================
// Memory Layout Tests
// =============================================================================

fn bench_memory_overhead_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_creation");
    
    for &size in &[SMALL_SIZE, MEDIUM_SIZE, LARGE_SIZE] {
        // Buffer creation
        group.bench_with_input(BenchmarkId::new("Buffer", size), &size, |b, &size| {
            b.iter(|| {
                let buffer = Buffer::<u32>::with_capacity(black_box(size));
                black_box(buffer);
            })
        });
        
        // Vec creation
        group.bench_with_input(BenchmarkId::new("Vec", size), &size, |b, &size| {
            b.iter(|| {
                let vec = Vec::<u32>::with_capacity(black_box(size));
                black_box(vec);
            })
        });
        
        // VecDeque creation
        group.bench_with_input(BenchmarkId::new("VecDeque", size), &size, |b, &size| {
            b.iter(|| {
                let deque = VecDeque::<u32>::with_capacity(black_box(size));
                black_box(deque);
            })
        });
    }
    
    group.finish();
}

// =============================================================================
// Iterator Performance
// =============================================================================

fn bench_iteration_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("iteration");
    
    for &size in &[SMALL_SIZE, MEDIUM_SIZE, LARGE_SIZE] {
        // Buffer iteration
        group.bench_with_input(BenchmarkId::new("Buffer", size), &size, |b, &size| {
            let mut buffer = Buffer::<u32>::with_capacity(size);
            for i in 0..size as u32 {
                buffer.push(i);
            }
            
            b.iter(|| {
                let data = buffer.data();
                let sum: u32 = data.iter().sum();
                black_box(sum);
            })
        });
        
        // Vec iteration
        group.bench_with_input(BenchmarkId::new("Vec", size), &size, |b, &_size| {
            let vec: Vec<u32> = (0..size as u32).collect();
            
            b.iter(|| {
                let sum: u32 = vec.iter().sum();
                black_box(sum);
            })
        });
    }
    
    group.finish();
}

criterion_group!(
    copy_benches, 
    bench_copy_from_slice_comparison,
    bench_copy_at_most_n_comparison
);

criterion_group!(
    sequential_benches,
    bench_sequential_push_comparison,
    bench_sequential_pop_comparison
);

criterion_group!(
    access_benches,
    bench_random_access_comparison,
    bench_iteration_comparison
);

criterion_group!(
    mutation_benches,
    bench_resize_comparison,
    bench_clear_comparison
);

criterion_group!(
    workload_benches,
    bench_mixed_workload_comparison
);

criterion_group!(
    creation_benches,
    bench_memory_overhead_comparison
);

criterion_main!(
    copy_benches,
    sequential_benches, 
    access_benches,
    mutation_benches,
    workload_benches,
    creation_benches
);
