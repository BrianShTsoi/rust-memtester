use {
    rust_memtester::{MemtestReportList, Memtester, MemtesterArgs},
    std::{
        mem::size_of,
        time::{Duration, Instant},
    },
};

const KB: usize = 1024;
const MB: usize = 1024 * KB;

fn main() -> anyhow::Result<()> {
    let start_time = Instant::now();
    let (mem_usize_count, memtester_args) = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(s) => {
            eprintln!(concat!(
                "Usage: rust-memtester ",
                "<memsize in MB> ",
                "<timeout in ms> ",
                "<allow_mem_resize as bool> ",
                "<allow_working_set_resize as bool> ",
                "<allow_multithread as bool> "
            ));
            anyhow::bail!("Invalid/missing argument '{s}'");
        }
    };
    let mut memory = vec![0; mem_usize_count];

    unsafe {
        println!("Creating memtester with: {memtester_args:#?}");
        let memtester = Memtester::all_tests_random_order(memtester_args);
        print_test_report_list(memtester.run(&mut memory)?)
    }

    println!();
    println!("Tester ran for {:?}", start_time.elapsed());
    Ok(())
}

fn parse_args<I, S>(mut iter: I) -> Result<(usize, MemtesterArgs), &'static str>
where
    I: Iterator<Item = S>,
    S: AsRef<str>,
{
    macro_rules! parse_next(($n: literal) => {
        iter.next().and_then(|s| s.as_ref().parse().ok()).ok_or($n)?
    });

    let memsize: usize = parse_next!("memsize");
    let mem_usize_count = memsize * MB / size_of::<usize>();
    let timeout = Duration::from_millis(parse_next!("timeout_ms"));

    Ok((
        mem_usize_count,
        MemtesterArgs {
            timeout,
            allow_mem_resize: parse_next!("allow_mem_resize"),
            allow_working_set_resize: parse_next!("allow_working_set_resize"),
            allow_multithread: parse_next!("allow_multithread"),
        },
    ))
}

fn print_test_report_list(report_list: MemtestReportList) {
    println!("Memtester ran successfully");
    println!("tested_memsize = {}", report_list.tested_usize_count);
    println!("mlocked = {}", report_list.mlocked);
    for report in report_list.reports {
        println!(
            "{:<30} {}",
            format!("Tested {:?}", report.test_type),
            format!("Outcome is {:?}", report.outcome)
        );
    }
}
