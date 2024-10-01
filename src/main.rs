use rust_memtester::{MemtestReportList, Memtester};
use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::env;
use std::time::Instant;

const KB: usize = 1024;
const MB: usize = 1024 * KB;

fn main() {
    let args: Vec<String> = env::args().collect();
    let exit_with_err = || {
        eprintln!(concat!(
            "Usage: cargo run ",
            "<mem_size in MB> ",
            "<timeout in ms> ",
            "<allow_mem_resize as bool> ",
            "<allow_working_set_resize as bool> ",
            "<allow_multithread as bool> "
        ));
        std::process::exit(1);
    };

    if args.len() != 6 {
        exit_with_err();
    }
    let memsize_mb = args[1].parse::<usize>().unwrap_or_else(|_| exit_with_err());
    let timeout = args[2].parse::<usize>().unwrap_or_else(|_| exit_with_err());
    let allow_mem_resize = args[3].parse::<usize>().unwrap_or_else(|_| exit_with_err()) != 0;
    let allow_working_set_resize =
        args[4].parse::<usize>().unwrap_or_else(|_| exit_with_err()) != 0;
    let allow_multithread = args[5].parse::<usize>().unwrap_or_else(|_| exit_with_err()) != 0;
    let memsize = memsize_mb * MB;

    let layout = Layout::from_size_align(memsize, 1);
    let Ok(layout) = layout else {
        println!("Failed to create layout");
        return;
    };

    let start_time = Instant::now();
    unsafe {
        let base_ptr = alloc(layout);
        if base_ptr.is_null() {
            handle_alloc_error(layout);
        }
        let memtester = Memtester::new(
            base_ptr,
            memsize,
            timeout,
            allow_mem_resize,
            allow_working_set_resize,
            allow_multithread,
        );
        print_memtester_input_parameters(
            base_ptr,
            memsize,
            timeout,
            allow_mem_resize,
            allow_working_set_resize,
            allow_multithread,
        );
        match memtester.run() {
            Ok(report_list) => {
                print_test_report_list(report_list);
            }
            Err(err) => {
                println!("{err:#?}")
            }
        }

        dealloc(base_ptr, layout);
    }
    println!();
    println!("Tester ran for {:?}", start_time.elapsed());
}

fn print_memtester_input_parameters(
    base_ptr: *mut u8,
    memsize: usize,
    timeout: usize,
    allow_mem_resize: bool,
    allow_working_set_resize: bool,
    allow_multithread: bool,
) {
    println!();
    println!("Created Memtester with ");
    println!("base_ptr = {base_ptr:?}");
    println!("memsize = {memsize}");
    println!("timeout = {timeout}");
    println!("allow_mem_resize = {allow_mem_resize}");
    println!("allow_working_set_resize = {allow_working_set_resize}");
    println!("allow_multithread = {allow_multithread}");
    println!();
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
