use rust_memtester::memtest;
use rust_memtester::Memtester;
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
    let memsize_in_bytes = memsize_mb * MB;

    let layout = Layout::from_size_align(memsize_in_bytes, 1);
    let Ok(layout) = layout else {
        println!("Failed to create layout");
        return;
    };

    unsafe {
        let base_ptr = alloc(layout);
        if base_ptr.is_null() {
            handle_alloc_error(layout);
        }
        let test_types = vec![
            memtest::MemtestType::TestOwnAddress,
            memtest::MemtestType::TestRandomValue,
        ];
        let memtester = Memtester::with_base_ptr(base_ptr, memsize_in_bytes, timeout, test_types);
        match memtester.run() {
            Ok(report) => {
                println!("{report:#?}");
            }
            Err(err) => {
                println!("{err:#?}")
            }
        }

        dealloc(base_ptr, layout);
    }
}
