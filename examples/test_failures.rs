use {
    anyhow::Context,
    rust_memtester::{Memtester, MemtesterArgs},
    std::{
        mem::size_of,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    },
    tracing::info,
    tracing_subscriber::fmt::format::FmtSpan,
};

#[derive(Copy, Clone)]
struct SendableVecPtr(*mut Vec<usize>);

unsafe impl Send for SendableVecPtr {}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(std::io::stderr)
        .with_thread_ids(true)
        .init();
    let start_time = Instant::now();
    let (mem_usize_count, memtester_args) = match parse_args() {
        Ok(parsed_args) => parsed_args,
        Err(s) => {
            eprintln!(concat!(
                "Usage: rust-memtester ",
                "<memsize in MB> ",
                "<timeout in ms> ",
                "<mem_lock_mode> ",
                "<allow_working_set_resize as bool> ",
                "<allow_multithread as bool> ",
                "<allow_early_temrmination as bool> "
            ));
            anyhow::bail!("Invalid/missing argument '{s}'");
        }
    };

    info!("Running memtester with: {memtester_args:#?}");
    let mut memory = vec![0; mem_usize_count];

    let test_complete = Arc::new(AtomicBool::new(false));
    let test_complete_clone = test_complete.clone();
    let memory_ptr = SendableVecPtr(&mut memory);

    // TODO: This can seg fault if memory is resized when running memtester
    let corrupt_memory_handle = thread::spawn(move || unsafe {
        while !test_complete.load(Ordering::Acquire) {
            const SLEEP_DURATION_MILLIS: u64 = 100;
            corrupt_random_memory(memory_ptr);
            thread::sleep(Duration::from_millis(SLEEP_DURATION_MILLIS));
        }
    });

    let memtester_handle = thread::spawn(move || {
        let test_result = Memtester::all_tests_random_order(&memtester_args).run(&mut memory);
        test_complete_clone.store(true, Ordering::Release);
        // Wait for the corrupt memory thread to end before dropping `memory`
        corrupt_memory_handle
            .join()
            .expect("corrupt memory thread panicked");
        test_result
    });

    let report_list = memtester_handle
        .join()
        .expect("memtester thread panicked")
        .context("Failed to run memtester")?;

    println!("Tester ran for {:?}", start_time.elapsed());
    println!("Test results: \n{report_list}");

    anyhow::ensure!(
        report_list.all_pass(),
        "Found failures or errors among memtest reports"
    );
    Ok(())
}

/// Parse command line arguments to return a usize for the requested memory vector length and
/// other memtester arguments
fn parse_args() -> Result<(usize, MemtesterArgs), &'static str> {
    const KB: usize = 1024;
    const MB: usize = 1024 * KB;

    let mut iter = std::env::args().skip(1);

    macro_rules! parse_next(($n: literal) => {
        iter.next().and_then(|s| s.parse().ok()).ok_or($n)?
    });

    let memsize: usize = parse_next!("memsize");
    let mem_usize_count = memsize * MB / size_of::<usize>();
    let timeout = Duration::from_millis(parse_next!("timeout_ms"));

    Ok((
        mem_usize_count,
        MemtesterArgs {
            timeout,
            mem_lock_mode: parse_next!("mem_lock_mode"),
            allow_working_set_resize: parse_next!("allow_working_set_resize"),
            allow_multithread: parse_next!("allow_multithread"),
            allow_early_termination: parse_next!("allow_early_termination"),
        },
    ))
}

unsafe fn corrupt_random_memory(memory: SendableVecPtr) {
    use rand::Rng;

    const CORRUPTION_VAL: usize = 0;
    let memory: &mut Vec<usize> = &mut *(memory.0);

    let n = rand::thread_rng().gen_range(0..memory.len());
    std::ptr::write_volatile(memory.as_mut_ptr().add(n), CORRUPTION_VAL)
}
