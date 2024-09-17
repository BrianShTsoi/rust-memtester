use memtest::{MemtestReport, MemtestType};
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::{io::ErrorKind, mem::size_of, time::Instant};

pub mod memtest;

#[derive(Debug)]
pub struct Memtester {
    base_ptr: *mut usize,
    memsize: usize,
    mem_usize_count: usize,
    timeout_ms: usize,
    test_types: Vec<MemtestType>,
}

#[derive(Debug)]
pub struct MemtestReportList {
    tested_memsize: usize,
    reports: Vec<MemtestReport>,
}

#[derive(Debug)]
pub enum MemtesterError {
    MemorylockFailure,
    WindowsWorkingSetFailure,
}

// TODO: get PAGE_SIZE from OS
const PAGE_SIZE: usize = 4096;

impl Memtester {
    // TODO: Memtester without given base_ptr, ie. take care of memory allocation as well
    // TODO: More configuration parameters:
    //       early termination? terminate per test vs all test?
    //       run speicific alogorithms, or random algorithms

    // NOTE: base_ptr may be moved to align with page boundaries
    // NOTE: memsize may be decremented for mlock
    /// Returns a Memtester containing all test types in random order
    pub fn new(base_ptr: *mut u8, memsize: usize, timeout_ms: usize) -> Memtester {
        let mut test_types = vec![
            MemtestType::TestOwnAddress,
            MemtestType::TestRandomVal,
            MemtestType::TestXor,
            MemtestType::TestSub,
            MemtestType::TestMul,
            MemtestType::TestDiv,
            MemtestType::TestOr,
            MemtestType::TestAnd,
            MemtestType::TestSeqInc,
            MemtestType::TestSolidBits,
            MemtestType::TestCheckerboard,
            MemtestType::TestBlockSeq,
        ];
        test_types.shuffle(&mut thread_rng());

        Memtester {
            base_ptr: base_ptr as *mut usize,
            memsize,
            mem_usize_count: memsize / size_of::<usize>(),
            timeout_ms,
            test_types,
        }
    }

    pub fn from_test_types(
        base_ptr: *mut u8,
        memsize: usize,
        timeout_ms: usize,
        test_types: Vec<MemtestType>,
    ) -> Memtester {
        Memtester {
            base_ptr: base_ptr as *mut usize,
            memsize,
            mem_usize_count: memsize / size_of::<usize>(),
            timeout_ms,
            test_types,
        }
    }

    pub unsafe fn run(mut self) -> Result<MemtestReportList, MemtesterError> {
        // TODO: memtester aligns base_ptr before mlock to avoid locking an extra page
        //       By default mlock rounds base_ptr down to nearest page boundary
        //       Not sure which is desirable
        // TODO: this way of alignment assumes memsize > PAGE_SIZE
        // let align_diff = self.base_ptr as usize % PAGE_SIZE;
        // self.base_ptr = unsafe { self.base_ptr.byte_add(PAGE_SIZE - align_diff) };
        // self.memsize -= PAGE_SIZE - align_diff;

        #[cfg(windows)]
        let (min_set_size, max_set_size) = win_working_set::replace_set_size(self.memsize)?;

        let mut lockguard = self.memory_resize_and_lock()?;

        let mut reports = Vec::new();
        let start_time = Instant::now();
        for test_type in &self.test_types {
            let mut add_report = |result| reports.push(MemtestReport::new(*test_type, result));
            match test_type {
                MemtestType::TestOwnAddress => add_report(self.test_own_address(start_time)),
                MemtestType::TestRandomVal => add_report(self.test_random_val(start_time)),
                MemtestType::TestXor => {
                    add_report(self.test_two_regions(start_time, Self::write_xor))
                }
                MemtestType::TestSub => {
                    add_report(self.test_two_regions(start_time, Self::write_sub))
                }
                MemtestType::TestMul => {
                    add_report(self.test_two_regions(start_time, Self::write_mul))
                }
                MemtestType::TestDiv => {
                    add_report(self.test_two_regions(start_time, Self::write_div))
                }
                MemtestType::TestOr => {
                    add_report(self.test_two_regions(start_time, Self::write_or))
                }
                MemtestType::TestAnd => {
                    add_report(self.test_two_regions(start_time, Self::write_and))
                }
                MemtestType::TestSeqInc => add_report(self.test_seq_inc(start_time)),
                MemtestType::TestSolidBits => add_report(self.test_solid_bits(start_time)),
                MemtestType::TestCheckerboard => add_report(self.test_checkerboard(start_time)),
                MemtestType::TestBlockSeq => add_report(self.test_block_seq(start_time)),
            }
        }

        drop(lockguard);
        #[cfg(windows)]
        {
            win_working_set::restore_set_size(min_set_size, max_set_size)?;
        }

        Ok(MemtestReportList {
            tested_memsize: self.memsize,
            reports,
        })
    }

    // TODO: At what point do we give up locking and proceed testing even with paging?
    fn memory_resize_and_lock(&mut self) -> Result<region::LockGuard, MemtesterError> {
        const WIN_OUTOFMEM_CODE: usize = 1453;
        loop {
            match region::lock(self.base_ptr, self.memsize) {
                Ok(lockguard) => {
                    self.mem_usize_count = self.memsize / size_of::<usize>();
                    return Ok(lockguard);
                }
                // TODO: macOS error?
                Err(region::Error::SystemCall(err))
                    if matches!(err.kind(), ErrorKind::OutOfMemory)
                        || err
                            .raw_os_error()
                            .is_some_and(|e| e as usize == WIN_OUTOFMEM_CODE) =>
                {
                    match self.memsize.checked_sub(PAGE_SIZE) {
                        Some(new_memsize) => self.memsize = new_memsize,
                        None => return Err(MemtesterError::MemorylockFailure),
                    }
                }
                _ => {
                    return Err(MemtesterError::MemorylockFailure);
                }
            }
        }
    }
}

#[cfg(windows)]
mod win_working_set {
    use super::MemtesterError;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessWorkingSetSize, SetProcessWorkingSetSize,
    };

    pub(super) unsafe fn replace_set_size(
        memsize: usize,
    ) -> Result<(usize, usize), MemtesterError> {
        let (mut min_set_size, mut max_set_size) = (0, 0);
        match GetProcessWorkingSetSize(GetCurrentProcess(), &mut min_set_size, &mut max_set_size) {
            Ok(()) => (),
            Err(_) => return Err(MemtesterError::WindowsWorkingSetFailure),
        }
        // TODO: Not sure what the best choice of min and max should be
        // TODO: handle overflow?
        match SetProcessWorkingSetSize(GetCurrentProcess(), memsize * 2, memsize * 4) {
            Ok(()) => (),
            Err(_) => return Err(MemtesterError::WindowsWorkingSetFailure),
        }
        Ok((min_set_size, max_set_size))
    }

    pub(super) unsafe fn restore_set_size(
        min_set_size: usize,
        max_set_size: usize,
    ) -> Result<(), MemtesterError> {
        match SetProcessWorkingSetSize(GetCurrentProcess(), min_set_size, max_set_size) {
            Ok(()) => (),
            Err(_) => return Err(MemtesterError::WindowsWorkingSetFailure),
        }
        Ok(())
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
