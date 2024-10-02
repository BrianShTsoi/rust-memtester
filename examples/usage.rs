use anyhow;
use rust_memtester::{MemtestReportList, Memtester, MemtesterArgs};
use std::mem::size_of;
use std::time::Instant;

const KB: usize = 1024;
const MB: usize = 1024 * KB;

fn main() -> anyhow::Result<()> {
    let start_time = Instant::now();

    let mut args = match MemtesterArgs::from_iter(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(s) => {
            eprintln!(concat!(
                "Usage: cargo run ",
                "<mem_size in MB> ",
                "<timeout in ms> ",
                "<allow_mem_resize as bool> ",
                "<allow_working_set_resize as bool> ",
                "<allow_multithread as bool> "
            ));
            anyhow::bail!("invalid/missing argument '{s}'");
        }
    };
    let mut vec = vec![0; args.memsize / size_of::<usize>()];
    args.base_ptr = vec.as_mut_ptr();

    unsafe {
        println!("Creating memtester with: {args:#?}");
        let memtester = Memtester::new(args);
        match memtester.run() {
            Ok(report_list) => {
                print_test_report_list(report_list);
            }
            Err(err) => {
                println!("Memtester had error {:#?}", err)
            }
        }
    }
    println!();
    println!("Tester ran for {:?}", start_time.elapsed());
    Ok(())
}

fn print_test_report_list(report_list: MemtestReportList) {
    println!("Memtester ran successfully");
    println!("tested_memsize = {}", report_list.tested_memsize);
    println!("mlocked = {}", report_list.mlocked);
    for report in report_list.reports {
        println!(
            "{:<30} {}",
            format!("Tested {:?}", report.test_type),
            format!("Outcome is {:?}", report.outcome)
        );
    }
}

trait MemtesterArgsExt: Sized {
    fn from_iter<I, S>(iter: I) -> Result<Self, &'static str>
    where
        I: Iterator<Item = S>,
        S: AsRef<str>;
}

impl MemtesterArgsExt for MemtesterArgs {
    fn from_iter<I, S>(mut iter: I) -> Result<Self, &'static str>
    where
        I: Iterator<Item = S>,
        S: AsRef<str>,
    {
        macro_rules! parse_next(($n: literal) => {
            iter.next().and_then(|s| s.as_ref().parse().ok()).ok_or($n)?
        });

        let mut memsize: usize = parse_next!("memsize");
        memsize *= MB;

        Ok(MemtesterArgs {
            base_ptr: std::ptr::null_mut(),
            memsize,
            timeout_ms: parse_next!("timeout_ms"),
            allow_mem_resize: parse_next!("allow_mem_resize"),
            allow_working_set_resize: parse_next!("allow_working_set_resize"),
            allow_multithread: parse_next!("allow_multithread"),
        })
    }
}
