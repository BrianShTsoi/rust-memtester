use rust_memtester::{MemtestReportList, Memtester};
use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::env;

const KB: usize = 1024;
const MB: usize = 1024 * KB;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        println!("Use: cargo run <mem_size in MB> <timeout in ms>");
        return;
    }
    let Ok(memsize_mb) = args[1].parse::<usize>() else {
        println!("Use: cargo run <mem_size in MB> <timeout in ms>");
        return;
    };
    let Ok(timeout) = args[2].parse::<usize>() else {
        println!("Use: cargo run <mem_size in MB> <timeout in ms>");
        return;
    };
    let memsize = memsize_mb * MB;

    let layout = Layout::from_size_align(memsize, 1);
    let Ok(layout) = layout else {
        println!("Failed to create layout");
        return;
    };

    unsafe {
        let base_ptr = alloc(layout);
        if base_ptr.is_null() {
            handle_alloc_error(layout);
        }
        let memtester = Memtester::new(base_ptr, memsize, timeout, true, false);
        print_memtester_input_parameters(base_ptr, memsize, timeout);
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
}

fn print_memtester_input_parameters(base_ptr: *mut u8, memsize: usize, timeout: usize) {
    println!();
    println!(
        "Created Memtester with base_ptr = {base_ptr:?}, memsize = {memsize}, timeout = {timeout}"
    );
    println!();
}

fn print_test_report_list(report_list: MemtestReportList) {
    println!("Memtester ran successfully");
    println!("tested_memsize is {}", report_list.tested_memsize);
    println!("mlocked is {}", report_list.mlocked);
    println!();
    for report in report_list.reports {
        println!(
            "{:<30} {}",
            format!("Tested {:?}", report.test_type),
            format!("Outcome is {:?}", report.outcome)
        );
    }
}
