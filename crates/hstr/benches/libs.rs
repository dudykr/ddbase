extern crate swc_node_base;

#[macro_use]
extern crate criterion;
use std::hash::Hash;

use compact_str::CompactString;
use criterion::{black_box, BatchSize, BenchmarkId, Criterion};
use rand::distributions::{Alphanumeric, DistString};
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use smartstring::{LazyCompact, SmartString};

macro_rules! string_creation {
    ($group:expr, $len:expr, $setup:expr) => {{
        let group = &mut $group;

        for len in $len {
            group.bench_with_input(BenchmarkId::new("kdy_str", len), &len, |b, _| {
                let mut store = kdy_str::AtomStore::default();

                b.iter_batched(
                    $setup(len),
                    |text| {
                        black_box(store.atom(text));
                    },
                    BatchSize::SmallInput,
                );
            });

            group.bench_with_input(BenchmarkId::new("kdy_str_slow", len), &len, |b, _| {
                b.iter_batched(
                    $setup(len),
                    |text| {
                        black_box(kdy_str::Atom::from(text));
                    },
                    BatchSize::SmallInput,
                );
            });

            group.bench_with_input(BenchmarkId::new("string_cache", len), &len, |b, _| {
                let mut prevent_drop = vec![];
                b.iter_batched(
                    $setup(len),
                    |text| {
                        let atom = black_box(string_cache::DefaultAtom::from(text));
                        prevent_drop.push(atom);
                    },
                    BatchSize::SmallInput,
                );
            });

            group.bench_with_input(BenchmarkId::new("compact_str", len), &len, |b, _| {
                b.iter_batched(
                    $setup(len),
                    |text| {
                        black_box(CompactString::from(text));
                    },
                    BatchSize::SmallInput,
                );
            });

            group.bench_with_input(BenchmarkId::new("smartstring", len), &len, |b, _| {
                b.iter_batched(
                    $setup(len),
                    |text| {
                        black_box(SmartString::<LazyCompact>::from(text));
                    },
                    BatchSize::SmallInput,
                );
            });

            group.bench_with_input(BenchmarkId::new("smol_str", len), &len, |b, _| {
                b.iter_batched(
                    $setup(len),
                    |text| {
                        black_box(smol_str::SmolStr::new(text));
                    },
                    BatchSize::SmallInput,
                );
            });

            group.bench_with_input(BenchmarkId::new("kstring", len), &len, |b, _| {
                b.iter_batched(
                    $setup(len),
                    |text| {
                        black_box(kstring::KString::from(text));
                    },
                    BatchSize::SmallInput,
                );
            });
        }
    }};
}

fn bench_basic_creation(c: &mut Criterion) {
    let length = [4usize, 8, 16, 32, 64, 128, 256, 512, 1024, 2048];

    {
        let mut group = c.benchmark_group(&format!("single-thread/create/cached"));

        string_creation!(group, length, |len| {
            let text = random_string(len);
            move || text.clone()
        });

        group.finish();
    }

    {
        let mut group = c.benchmark_group(&format!("single-thread/create/not-cached"));

        string_creation!(group, length, |len| { move || random_string(len) });

        group.finish();
    }

    {
        let mut group = c.benchmark_group(&format!("single-thread/create/mixed"));

        string_creation!(group, length, |len| {
            let text = random_string(len);
            let mut i = 0;
            move || {
                i += 1;

                if i % 5 == 0 {
                    text.clone()
                } else {
                    random_string(len)
                }
            }
        });

        group.finish();
    }
}

fn bench_hash_operation(c: &mut Criterion) {
    fn random_hashset<S>(items: Vec<S>) -> FxHashSet<S>
    where
        S: Eq + Hash,
    {
        let mut set = FxHashSet::default();

        set.extend(items);

        set
    }

    fn random_keys<S>(len: usize, convert: &mut dyn FnMut(String) -> S) -> Vec<S> {
        (0..len)
            .into_iter()
            .map(|_| random_string(1024))
            .map(|s| convert(s))
            .collect()
    }

    fn prepare<S>(len: usize, convert: &mut dyn FnMut(String) -> S) -> (FxHashSet<S>, Vec<S>)
    where
        S: Eq + Hash + Clone,
    {
        let items = random_keys(len, convert);

        let set = random_hashset(items.clone());
        let mut keys = random_keys(9 * len, &mut *convert);

        keys.extend(items);

        (set, keys)
    }

    let mut group = c.benchmark_group(&format!("single-thread/HashSet"));

    let length = [1000, 10000];

    {
        for len in length {
            group.bench_with_input(BenchmarkId::new("kdy_str", len), &len, |b, _| {
                let mut for_fairness = vec![];
                let mut store = kdy_str::AtomStore::default();
                b.iter_batched(
                    || prepare(len, &mut |s| store.atom(s)),
                    |(map, keys)| {
                        for key in &keys {
                            black_box(map.contains(&key));
                        }
                        for_fairness.extend(map);
                        for_fairness.extend(keys);
                    },
                    BatchSize::SmallInput,
                );
            });
            group.bench_with_input(BenchmarkId::new("string_cache", len), &len, |b, _| {
                let mut for_fairness = vec![];
                b.iter_batched(
                    || prepare(len, &mut string_cache::DefaultAtom::from),
                    |(map, keys)| {
                        for key in &keys {
                            black_box(map.contains(key));
                        }
                        for_fairness.extend(map);
                        for_fairness.extend(keys);
                    },
                    BatchSize::SmallInput,
                );
            });
            group.bench_with_input(BenchmarkId::new("compact_str", len), &len, |b, _| {
                b.iter_batched(
                    || prepare(len, &mut CompactString::from),
                    |(map, keys)| {
                        for key in keys {
                            black_box(map.contains(&key));
                        }
                    },
                    BatchSize::SmallInput,
                );
            });
            group.bench_with_input(BenchmarkId::new("smartstring", len), &len, |b, _| {
                b.iter_batched(
                    || prepare(len, &mut SmartString::<LazyCompact>::from),
                    |(map, keys)| {
                        for key in keys {
                            black_box(map.contains(&key));
                        }
                    },
                    BatchSize::SmallInput,
                );
            });
            group.bench_with_input(BenchmarkId::new("smol_str", len), &len, |b, _| {
                b.iter_batched(
                    || prepare(len, &mut smol_str::SmolStr::from),
                    |(map, keys)| {
                        for key in keys {
                            black_box(map.contains(&key));
                        }
                    },
                    BatchSize::SmallInput,
                );
            });
            group.bench_with_input(BenchmarkId::new("kstring", len), &len, |b, _| {
                b.iter_batched(
                    || prepare(len, &mut kstring::KString::from),
                    |(map, keys)| {
                        for key in keys {
                            black_box(map.contains(&key));
                        }
                    },
                    BatchSize::SmallInput,
                );
            });
        }
    };

    group.finish();
}

fn bench_parallel_creation(c: &mut Criterion) {
    {
        let mut group = c.benchmark_group(&format!("parallel/create"));

        for len in [64, 256, 1024] {
            group.bench_with_input(BenchmarkId::new("kdy_str", len), &len, |b, _| {
                b.iter_batched(
                    || {
                        (0..num_cpus::get())
                            .into_iter()
                            .map(|_| kdy_str::AtomStore::default())
                            .collect::<Vec<_>>()
                    },
                    |stores| {
                        let mut main_store = kdy_str::AtomStore::default();

                        stores
                            .into_par_iter()
                            .map(|mut store| {
                                let atoms = (0..len)
                                    .into_iter()
                                    .map(|_| store.atom(random_string(65)))
                                    .collect::<Vec<_>>();

                                (store, atoms)
                            })
                            .collect::<Vec<_>>()
                            .into_iter()
                            .for_each(|(store, atoms)| {
                                black_box(atoms);
                                main_store.merge(store);
                            });
                    },
                    BatchSize::SmallInput,
                );
            });

            group.bench_with_input(BenchmarkId::new("string_cache", len), &len, |b, _| {
                b.iter_batched(
                    || {},
                    |_| {
                        let mut main_store = vec![];
                        (0..num_cpus::get())
                            .into_par_iter()
                            .map(|_| {
                                (0..len)
                                    .into_iter()
                                    .map(|_| {
                                        black_box(string_cache::DefaultAtom::from(random_string(
                                            65,
                                        )))
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .collect::<Vec<_>>()
                            .into_iter()
                            .for_each(|v| {
                                main_store.extend(black_box(v));
                            });
                    },
                    BatchSize::SmallInput,
                );
            });
        }

        group.finish();
    }
}

criterion_group!(
    benches,
    bench_basic_creation,
    bench_parallel_creation,
    bench_hash_operation
);
criterion_main!(benches);

fn random_string(len: usize) -> String {
    Alphanumeric.sample_string(&mut rand::thread_rng(), len)
}
