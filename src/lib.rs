use memtest::MemtestReport;
use region;
use std::{mem::size_of, time::Instant};

pub mod memtest;

#[derive(Debug)]
pub struct Memtester {
    base_ptr: *mut usize,
    memcount: usize,
    timeout_ms: usize,
    test_types: Vec<memtest::MemtestType>,
}

#[derive(Debug)]
pub struct MemtestReportList {
    reports: Vec<MemtestReport>,
}

#[derive(Debug)]
pub enum MemtesterError {
    Error,
}

impl Memtester {
    // TODO: Memtester without given base_ptr, ie. take care of memory allocation as well
    // TODO: More configuration parameters:
    //       early termination? terminate per test vs all test?
    //       run speicific alogorithms, or random algorithms
    pub fn with_base_ptr(
        base_ptr: *mut u8,
        memsize_in_bytes: usize,
        timeout_ms: usize,
        test_types: Vec<memtest::MemtestType>,
    ) -> Memtester {
        Memtester {
            base_ptr: base_ptr as *mut usize,
            memcount: memsize_in_bytes / size_of::<usize>(),
            timeout_ms,
            test_types,
        }
    }

    pub fn run(&self) -> Result<MemtestReportList, MemtesterError> {
        // TODO: handle mlock failures. lock less? give up locking?
        let _lockguard =
            region::lock(self.base_ptr, self.memcount).expect("mlock should be successful");
        let mut reports = Vec::new();
        let start_time = Instant::now();
        unsafe {
            use memtest::MemtestType;
            for test_type in &self.test_types {
                match test_type {
                    MemtestType::TestOwnAddress => reports.push(MemtestReport::new(
                        *test_type,
                        self.test_own_address(start_time),
                    )),
                    MemtestType::TestRandomValue => reports.push(MemtestReport::new(
                        *test_type,
                        self.test_random_value(start_time),
                    )),
                }
            }
            // reports.push(self.test_own_address(start_time));
        }
        Ok(MemtestReportList { reports })
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    #[test]
    fn test1() {
        assert!(true);
    }
}
