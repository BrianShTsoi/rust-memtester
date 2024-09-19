use memtest::{MemtestReport, MemtestType};
use rand::{seq::SliceRandom, thread_rng};
use std::{
    io::ErrorKind,
    mem::size_of,
    time::{Duration, Instant},
};

pub mod memtest;

#[derive(Debug)]
pub struct Memtester {
    base_ptr: *mut usize,
    memsize: usize,
    timeout_ms: usize,
    allow_working_set_resize: bool,
    allow_mem_resize: bool,
    test_types: Vec<MemtestType>,
}

#[derive(Debug)]
pub struct MemtestReportList {
    pub tested_memsize: usize,
    pub mlocked: bool,
    pub reports: Vec<MemtestReport>,
}

#[derive(Debug)]
pub enum MemtesterError {
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
    // NOTE: memsize should be a multple of size_of::<usize>() to avoid remainder bytes
    /// Returns a Memtester containing all test types in random order
    pub fn new(
        base_ptr: *mut u8,
        memsize: usize,
        timeout_ms: usize,
        allow_mem_resize: bool,
        allow_working_set_resize: bool,
    ) -> Memtester {
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
            timeout_ms,
            allow_working_set_resize,
            allow_mem_resize,
            test_types,
        }
    }

    pub fn from_test_types(
        base_ptr: *mut u8,
        memsize: usize,
        timeout_ms: usize,
        test_types: Vec<MemtestType>,
        allow_mem_resize: bool,
        allow_working_set_resize: bool,
    ) -> Memtester {
        Memtester {
            base_ptr: base_ptr as *mut usize,
            memsize,
            timeout_ms,
            allow_working_set_resize,
            allow_mem_resize,
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
        let working_set_sizes = if self.allow_working_set_resize {
            Some(win_working_set::replace_set_size(self.memsize)?)
        } else {
            None
        };

        let lockguard = self.memory_resize_and_lock().ok();
        let mlocked = lockguard.is_some();

        let tested_memsize = self.memsize / size_of::<usize>();

        let start_time = Instant::now();
        let mut reports = Vec::new();
        for test_type in &self.test_types {
            let test = match test_type {
                MemtestType::TestOwnAddress => memtest::test_own_address,
                MemtestType::TestRandomVal => memtest::test_random_val,
                MemtestType::TestXor => memtest::test_xor,
                MemtestType::TestSub => memtest::test_sub,
                MemtestType::TestMul => memtest::test_mul,
                MemtestType::TestDiv => memtest::test_div,
                MemtestType::TestOr => memtest::test_or,
                MemtestType::TestAnd => memtest::test_and,
                MemtestType::TestSeqInc => memtest::test_seq_inc,
                MemtestType::TestSolidBits => memtest::test_solid_bits,
                MemtestType::TestCheckerboard => memtest::test_checkerboard,
                MemtestType::TestBlockSeq => memtest::test_block_seq,
            };
            // TODO: casting u128 to usize is unideal?
            let time_left = Duration::from_millis(self.timeout_ms as u64)
                .saturating_sub(start_time.elapsed())
                .as_millis() as usize;
            let test_result = if time_left > 0 {
                test(self.base_ptr, tested_memsize, time_left)
            } else {
                Err(memtest::MemtestError::Timeout)
            };

            reports.push(MemtestReport::new(*test_type, test_result));
        }

        drop(lockguard);

        #[cfg(windows)]
        if let Some((min_set_size, max_set_size)) = working_set_sizes {
            win_working_set::restore_set_size(min_set_size, max_set_size)?;
        }

        Ok(MemtestReportList {
            tested_memsize,
            mlocked,
            reports,
        })
    }

    fn memory_resize_and_lock(&mut self) -> Result<region::LockGuard, ()> {
        const WIN_OUTOFMEM_CODE: usize = 1453;
        loop {
            match region::lock(self.base_ptr, self.memsize) {
                Ok(lockguard) => {
                    return Ok(lockguard);
                }
                // TODO: macOS error?
                Err(region::Error::SystemCall(err))
                    if ((matches!(err.kind(), ErrorKind::OutOfMemory)
                        || err
                            .raw_os_error()
                            .is_some_and(|e| e as usize == WIN_OUTOFMEM_CODE))
                        && self.allow_mem_resize) =>
                {
                    match self.memsize.checked_sub(PAGE_SIZE) {
                        Some(new_memsize) => self.memsize = new_memsize,
                        None => return Err(()),
                    }
                }
                _ => {
                    return Err(());
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
        match SetProcessWorkingSetSize(
            GetCurrentProcess(),
            memsize.saturating_mul(2),
            memsize.saturating_mul(4),
        ) {
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
            // TODO: at this point all tests are run, returning an Error is unideal
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
