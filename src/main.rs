use rust_memtester::{MemtestReportList, Memtester};
use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::env;

const KB: usize = 1024;
const MB: usize = 1024 * KB;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 5 {
        println!("Use: cargo run <mem_size in MB> <timeout in ms> <allow_mem_resize as bool> <allow_working_set_resize as bool>");
        return;
    }
    let Ok(memsize_mb) = args[1].parse::<usize>() else {
        println!("Use: cargo run <mem_size in MB> <timeout in ms> <allow_mem_resize as bool> <allow_working_set_resize as bool>");
        return;
    };
    let Ok(timeout) = args[2].parse::<usize>() else {
        println!("Use: cargo run <mem_size in MB> <timeout in ms> <allow_mem_resize as bool> <allow_working_set_resize as bool>");
        return;
    };
    let Ok(allow_mem_resize) = args[3].parse::<usize>() else {
        println!("Use: cargo run <mem_size in MB> <timeout in ms> <allow_mem_resize as bool> <allow_working_set_resize as bool>");
        return;
    };
    let allow_mem_resize = allow_mem_resize != 0;
    let Ok(allow_working_set_resize) = args[4].parse::<usize>() else {
        println!("Use: cargo run <mem_size in MB> <timeout in ms> <allow_mem_resize as bool> <allow_working_set_resize as bool>");
        return;
    };
    let allow_working_set_resize = allow_working_set_resize != 0;
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
        let memtester = Memtester::new(
            base_ptr,
            memsize,
            timeout,
            allow_mem_resize,
            allow_working_set_resize,
        );
        print_memtester_input_parameters(
            base_ptr,
            memsize,
            timeout,
            allow_mem_resize,
            allow_working_set_resize,
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
}

fn print_memtester_input_parameters(
    base_ptr: *mut u8,
    memsize: usize,
    timeout: usize,
    allow_mem_resize: bool,
    allow_working_set_resize: bool,
) {
    println!();
    println!("Created Memtester with ");
    println!("base_ptr = {base_ptr:?}");
    println!("memsize = {memsize}");
    println!("timeout = {timeout}");
    println!("allow_mem_resize = {allow_mem_resize}");
    println!("allow_working_set_resize = {allow_working_set_resize}");
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
