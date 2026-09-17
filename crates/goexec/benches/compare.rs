//! Standalone, descriptive benchmark; deliberately has no CI speed assertions.
use std::{
    hint::black_box,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
enum Case {
    Yield,
    Empty,
    CachedRead,
    SyscallWait,
    Cpu,
    Mixed,
}

#[derive(Clone, Copy)]
enum Op {
    Yield,
    Empty,
    Read,
    Wait,
    Cpu,
}

impl Case {
    fn operation(self, iteration: usize) -> Op {
        match self {
            Self::Yield => Op::Yield,
            Self::Empty => Op::Empty,
            Self::CachedRead => Op::Read,
            Self::SyscallWait => Op::Wait,
            Self::Cpu => Op::Cpu,
            Self::Mixed => match iteration % 4 {
                0 => Op::Wait,
                1 => Op::Read,
                _ => Op::Cpu,
            },
        }
    }
}

struct Context {
    path: PathBuf,
    socket: Option<TcpStream>,
}

impl Context {
    fn run(&mut self, operation: Op) {
        match operation {
            Op::Yield => {}
            Op::Empty => black_box(()),
            Op::Read => {
                black_box(std::fs::read(&self.path).unwrap());
            }
            Op::Wait => {
                let socket = self.socket.as_mut().unwrap();
                socket.write_all(&[1]).unwrap();
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                black_box(byte);
            }
            Op::Cpu => {
                let mut value = black_box(17u64);
                for _ in 0..10_000 {
                    value = value.wrapping_mul(6364136223846793005).rotate_left(7);
                }
                black_box(value);
            }
        }
    }
}

// Peers are load generators, excluded from executor thread counts and timing.
// A blocking socket read waits for a peer's delayed reply, i.e. a real syscall.
fn contexts(
    case: Case,
    lanes: usize,
    path: &std::path::Path,
) -> (Vec<Context>, Vec<thread::JoinHandle<()>>) {
    let mut contexts = Vec::new();
    let mut peers = Vec::new();
    for _ in 0..lanes {
        let socket = if matches!(case, Case::SyscallWait | Case::Mixed) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut peer, _) = listener.accept().unwrap();
            socket.set_nodelay(true).unwrap();
            peer.set_nodelay(true).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            peers.push(thread::spawn(move || {
                let mut byte = [0];
                while peer.read_exact(&mut byte).is_ok() {
                    thread::sleep(Duration::from_micros(500));
                    peer.write_all(&byte).unwrap();
                }
            }));
            Some(socket)
        } else {
            None
        };
        contexts.push(Context {
            path: path.into(),
            socket,
        });
    }
    (contexts, peers)
}

type Job = Box<dyn FnOnce() + Send>;

struct SyncPool {
    sender: Option<mpsc::Sender<Job>>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl SyncPool {
    fn new(parallelism: usize) -> Self {
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        let threads = (0..parallelism)
            .map(|_| {
                let receiver = receiver.clone();
                thread::spawn(move || loop {
                    let job = receiver.lock().unwrap().recv();
                    match job {
                        Ok(job) => job(),
                        Err(_) => break,
                    }
                })
            })
            .collect();
        Self {
            sender: Some(sender),
            threads,
        }
    }
}

impl Drop for SyncPool {
    fn drop(&mut self) {
        self.sender.take();
        for thread in self.threads.drain(..) {
            thread.join().unwrap();
        }
    }
}

#[derive(Default)]
struct ThreadCounts {
    live: AtomicUsize,
    peak: AtomicUsize,
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Direct,
    Goexec,
    SpawnBlocking,
    BlockInPlace,
    Go,
}

enum Engine {
    Direct(SyncPool),
    Goexec(goexec::Runtime),
    Tokio(tokio::runtime::Runtime, Mode, Arc<ThreadCounts>),
}

impl Engine {
    fn new(mode: Mode, parallelism: usize, max_threads: usize) -> Self {
        match mode {
            Mode::Go => unreachable!("Go runs in its own process"),
            Mode::Direct => Self::Direct(SyncPool::new(parallelism)),
            Mode::Goexec => Self::Goexec(
                goexec::Runtime::builder()
                    .parallelism(parallelism)
                    .max_threads(max_threads)
                    .build()
                    .unwrap(),
            ),
            Mode::SpawnBlocking | Mode::BlockInPlace => {
                let counts = Arc::new(ThreadCounts::default());
                let started = counts.clone();
                let stopped = counts.clone();
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(parallelism)
                    // Tokio's cap is additional blocking threads; goexec's cap
                    // includes its regular workers. Match the total OS cap.
                    .max_blocking_threads(max_threads - parallelism)
                    .on_thread_start(move || {
                        let live = started.live.fetch_add(1, Ordering::Relaxed) + 1;
                        started.peak.fetch_max(live, Ordering::Relaxed);
                    })
                    .on_thread_stop(move || {
                        stopped.live.fetch_sub(1, Ordering::Relaxed);
                    })
                    .build()
                    .unwrap();
                Self::Tokio(rt, mode, counts)
            }
        }
    }

    fn counters(&self) -> (usize, Option<u64>) {
        match self {
            Self::Direct(pool) => (pool.threads.len(), None),
            Self::Goexec(rt) => {
                let metrics = rt.metrics();
                (metrics.spawned_threads, Some(metrics.handoffs))
            }
            Self::Tokio(_, _, counts) => (counts.peak.load(Ordering::Relaxed), None),
        }
    }

    fn run(
        &self,
        case: Case,
        contexts: Vec<Context>,
        iterations: usize,
        latency: bool,
    ) -> Vec<Duration> {
        match self {
            Self::Direct(pool) => {
                let (sender, receiver) = mpsc::channel();
                let lanes = contexts.len();
                for mut context in contexts {
                    let sender = sender.clone();
                    pool.sender
                        .as_ref()
                        .unwrap()
                        .send(Box::new(move || {
                            let mut samples =
                                Vec::with_capacity(if latency { iterations } else { 0 });
                            for iteration in 0..iterations {
                                let start = latency.then(Instant::now);
                                context.run(case.operation(iteration));
                                if let Some(start) = start {
                                    samples.push(start.elapsed());
                                }
                            }
                            sender.send(samples).unwrap();
                        }))
                        .unwrap();
                }
                (0..lanes).flat_map(|_| receiver.recv().unwrap()).collect()
            }
            Self::Goexec(rt) => {
                let tasks: Vec<_> = contexts
                    .into_iter()
                    .map(|mut context| {
                        rt.spawn(async move {
                            let mut samples =
                                Vec::with_capacity(if latency { iterations } else { 0 });
                            for iteration in 0..iterations {
                                let operation = case.operation(iteration);
                                let start = latency.then(Instant::now);
                                if matches!(operation, Op::Cpu | Op::Yield) {
                                    context.run(operation);
                                } else {
                                    goexec::blocking(|| context.run(operation));
                                }
                                if let Some(start) = start {
                                    samples.push(start.elapsed());
                                }
                                goexec::yield_now().await;
                            }
                            samples
                        })
                    })
                    .collect();
                futures::executor::block_on(async {
                    let mut samples = Vec::new();
                    for task in tasks {
                        samples.extend(task.await.unwrap());
                    }
                    samples
                })
            }
            Self::Tokio(rt, mode, _) => {
                let mode = *mode;
                let tasks: Vec<_> = contexts
                    .into_iter()
                    .map(|mut context| {
                        rt.spawn(async move {
                            let mut samples =
                                Vec::with_capacity(if latency { iterations } else { 0 });
                            for iteration in 0..iterations {
                                let operation = case.operation(iteration);
                                let start = latency.then(Instant::now);
                                if matches!(operation, Op::Cpu | Op::Yield) {
                                    // CPU work stays on executor workers for all modes.
                                    context.run(operation);
                                } else if matches!(mode, Mode::SpawnBlocking) {
                                    context = tokio::task::spawn_blocking(move || {
                                        context.run(operation);
                                        context
                                    })
                                    .await
                                    .unwrap();
                                } else {
                                    tokio::task::block_in_place(|| context.run(operation));
                                }
                                if let Some(start) = start {
                                    samples.push(start.elapsed());
                                }
                                tokio::task::yield_now().await;
                            }
                            samples
                        })
                    })
                    .collect();
                rt.block_on(async {
                    let mut samples = Vec::new();
                    for task in tasks {
                        samples.extend(task.await.unwrap());
                    }
                    samples
                })
            }
        }
    }
}

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Keep the delayed peers out of Go's GOMAXPROCS budget as well as its timing.
// Go opens a fresh connection per lane for both warmup and measurement.
fn run_go(
    binary: &std::path::Path,
    case: Case,
    path: &std::path::Path,
    parallelism: usize,
    lanes: usize,
    iterations: usize,
    latency: bool,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let stopped = done.clone();
    let connections = if matches!(case, Case::SyscallWait | Case::Mixed) {
        2 * lanes // Warmup and measured contexts.
    } else {
        0
    };
    let server = thread::spawn(move || {
        let mut peers = Vec::new();
        // Once setup is complete, join the peers instead of periodically
        // waking an accept loop during Go's timed region.
        while peers.len() < connections && !stopped.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut peer, _)) => {
                    // Accepted sockets inherit O_NONBLOCK on some Unix hosts.
                    peer.set_nonblocking(false).unwrap();
                    peer.set_nodelay(true).unwrap();
                    peer.set_read_timeout(Some(Duration::from_secs(30)))
                        .unwrap();
                    peers.push(thread::spawn(move || {
                        let mut byte = [0];
                        while peer.read_exact(&mut byte).is_ok() {
                            thread::sleep(Duration::from_micros(500));
                            if peer.write_all(&byte).is_err() {
                                break;
                            }
                        }
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("accept: {error}"),
            }
        }
        for peer in peers {
            peer.join().unwrap();
        }
    });
    let status = Command::new(binary)
        .args([
            "--case",
            &format!("{case:?}"),
            "--parallelism",
            &parallelism.to_string(),
            "--lanes",
            &lanes.to_string(),
            "--iterations",
            &iterations.to_string(),
            "--peer",
            &address.to_string(),
            &format!("--latency={latency}"),
            "--path",
        ])
        .arg(path)
        .status();
    done.store(true, Ordering::Release);
    server.join().unwrap();
    assert!(
        status.expect("start Go benchmark").success(),
        "Go benchmark failed"
    );
}

fn main() {
    let mut iterations = 2_000usize;
    let mut parallelism = thread::available_parallelism().map_or(1, usize::from);
    let mut lanes = None;
    let mut latency = true;
    let mut go_binary = None;
    let mut cases = vec![
        Case::Empty,
        Case::CachedRead,
        Case::SyscallWait,
        Case::Cpu,
        Case::Mixed,
    ];
    let mut modes = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--iterations" => iterations = args.next().unwrap().parse().unwrap(),
            "--parallelism" => parallelism = args.next().unwrap().parse().unwrap(),
            "--lanes" => lanes = Some(args.next().unwrap().parse::<usize>().unwrap()),
            "--no-latency" => latency = false,
            "--go-binary" => go_binary = Some(PathBuf::from(args.next().unwrap())),
            "--cases" => {
                cases = args
                    .next()
                    .unwrap()
                    .split(',')
                    .map(|case| match case {
                        "Yield" => Case::Yield,
                        "Empty" => Case::Empty,
                        "CachedRead" => Case::CachedRead,
                        "SyscallWait" => Case::SyscallWait,
                        "Cpu" => Case::Cpu,
                        "Mixed" => Case::Mixed,
                        _ => panic!("unknown case {case}"),
                    })
                    .collect();
            }
            "--modes" => {
                modes = Some(
                    args.next()
                        .unwrap()
                        .split(',')
                        .map(|mode| match mode {
                            "Direct" => Mode::Direct,
                            "Goexec" => Mode::Goexec,
                            "SpawnBlocking" => Mode::SpawnBlocking,
                            "BlockInPlace" => Mode::BlockInPlace,
                            "Go" => Mode::Go,
                            _ => panic!("unknown mode {mode}"),
                        })
                        .collect::<Vec<_>>(),
                );
            }
            "--bench" => {}
            "--help" | "-h" => {
                eprintln!(
                    "compare [--iterations 2000] [--parallelism CPUs] [--lanes 4*parallelism] \
                     [--no-latency] [--go-binary PATH]\n[--cases \
                     Yield,Empty,CachedRead,SyscallWait,Cpu,Mixed]\n[--modes \
                     Direct,Goexec,SpawnBlocking,BlockInPlace,Go]\nCSV on stdout; progress and \
                     configuration on stderr."
                );
                return;
            }
            _ => panic!("unknown argument {arg}"),
        }
    }
    let lanes = lanes.unwrap_or(parallelism * 4);
    assert!(iterations > 0 && parallelism > 0 && lanes > 0);
    let max_threads = 512.max(parallelism + 1);
    let modes = modes.unwrap_or_else(|| {
        let mut modes = vec![
            Mode::Direct,
            Mode::Goexec,
            Mode::SpawnBlocking,
            Mode::BlockInPlace,
        ];
        if go_binary.is_some() {
            modes.push(Mode::Go);
        }
        modes
    });
    assert!(
        !modes.iter().any(|mode| matches!(mode, Mode::Go)) || go_binary.is_some(),
        "--modes Go requires --go-binary PATH"
    );
    let directory = std::env::temp_dir().join(format!("goexec-bench-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let fixture = Fixture(directory);
    let path = fixture.0.join("cached.bin");
    std::fs::write(&path, vec![42; 4096]).unwrap();
    black_box(std::fs::read(&path).unwrap());
    eprintln!(
        "parallelism={parallelism}, lanes={lanes}, iterations/lane={iterations}, \
         max_threads={max_threads}, latency={latency}, reply_delay=500us; peer threads excluded"
    );
    println!(
        "engine,workload,phase,operations,elapsed_us,ops_per_sec,p50_ns,p99_ns,peak_workers,\
         handoffs"
    );
    for case in cases {
        for &mode in &modes {
            eprintln!("{mode:?} {case:?}");
            if matches!(mode, Mode::Go) {
                std::io::stdout().flush().unwrap();
                run_go(
                    go_binary.as_ref().unwrap(),
                    case,
                    &path,
                    parallelism,
                    lanes,
                    iterations,
                    latency,
                );
                continue;
            }
            let init = Instant::now();
            let engine = Engine::new(mode, parallelism, max_threads);
            let init_time = init.elapsed();
            let (workers, _) = engine.counters();
            println!(
                "{mode:?},{case:?},init,0,{},,,,{},",
                init_time.as_micros(),
                workers
            );
            let (warm, peers) = contexts(case, lanes, &path);
            black_box(engine.run(case, warm, iterations.min(32), latency));
            for peer in peers {
                peer.join().unwrap();
            }
            let (contexts, peers) = contexts(case, lanes, &path);
            let (_, before) = engine.counters();
            let start = Instant::now();
            let mut samples = engine.run(case, contexts, iterations, latency);
            let elapsed = start.elapsed();
            let (workers, after) = engine.counters();
            for peer in peers {
                peer.join().unwrap();
            }
            samples.sort_unstable();
            let quantile = |percent: usize| {
                if samples.is_empty() {
                    String::new()
                } else {
                    samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)]
                        .as_nanos()
                        .to_string()
                }
            };
            let handoffs = after
                .zip(before)
                .map(|(after, before)| (after - before).to_string())
                .unwrap_or_default();
            println!(
                "{mode:?},{case:?},steady,{},{},{:.2},{},{},{workers},{handoffs}",
                lanes * iterations,
                elapsed.as_micros(),
                (lanes * iterations) as f64 / elapsed.as_secs_f64(),
                quantile(50),
                quantile(99)
            );
        }
    }
}
