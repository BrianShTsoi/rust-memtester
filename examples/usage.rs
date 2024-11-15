use {
    rust_memtester::{Memtester, MemtesterArgs},
    std::{
        mem::size_of,
        time::{Duration, Instant},
    },
    tracing::info,
    tracing_subscriber::fmt::format::FmtSpan,
};

// TODO: Command line option for json output
// TODO: Command line option for specified tests?
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
    let report_list = Memtester::all_tests_random_order(memtester_args).run(&mut memory)?;
    println!("Tester ran for {:?}", start_time.elapsed());
    println!("Test results: \n{report_list}");

    anyhow::ensure!(
        report_list.all_pass(),
        "Found failures or errors among memtest reports"
    );
    Ok(())
}

/// Parse the iter and return a usize for the requested memory vector length and other memtester argumentes
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
