use memtest::{MemtestError, MemtestOutcome, MemtestReport, MemtestType};
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
    mem_usize_count: usize,
    timeout_ms: usize,
    allow_working_set_resize: bool,
    allow_mem_resize: bool,
    allow_multithread: bool,
    test_types: Vec<MemtestType>,
}

#[derive(Debug)]
pub struct MemtesterArgs {
    pub base_ptr: *mut usize,
    pub mem_usize_count: usize,
    pub timeout_ms: usize,
    pub allow_working_set_resize: bool,
    pub allow_mem_resize: bool,
    pub allow_multithread: bool,
}

#[derive(Debug)]
pub struct MemtestReportList {
    pub tested_usize_count: usize,
    pub mlocked: bool,
    pub reports: Vec<MemtestReport>,
}

#[derive(Debug)]
pub enum MemtesterError {
    WindowsWorkingSetFailure,
}

impl Memtester {
    // TODO: Memtester without given base_ptr, ie. take care of memory allocation as well
    // TODO: More configuration parameters:
    //       early termination? terminate per test vs all test?

    // NOTE: `mem_usize_count` may be decremented for mlock
    /// Create a Memtester containing all test types in random order
    pub fn new(args: MemtesterArgs) -> Memtester {
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
            base_ptr: args.base_ptr,
            mem_usize_count: args.mem_usize_count,
            timeout_ms: args.timeout_ms,
            allow_working_set_resize: args.allow_working_set_resize,
            allow_mem_resize: args.allow_mem_resize,
            allow_multithread: args.allow_multithread,
            test_types,
        }
    }

    /// Create a Memtester with specified test types
    pub fn from_test_types(args: MemtesterArgs, test_types: Vec<MemtestType>) -> Memtester {
        Memtester {
            base_ptr: args.base_ptr,
            mem_usize_count: args.mem_usize_count,
            timeout_ms: args.timeout_ms,
            allow_working_set_resize: args.allow_working_set_resize,
            allow_mem_resize: args.allow_mem_resize,
            allow_multithread: args.allow_multithread,
            test_types,
        }
    }

    /// Consume the Memtester and run the tests
    pub unsafe fn run(mut self) -> Result<MemtestReportList, MemtesterError> {
        let start_time = Instant::now();
        // TODO: the linux memtester aligns base_ptr before mlock to avoid locking an extra page
        //       By default mlock rounds base_ptr down to nearest page boundary
        //       Not sure which is desirable

        #[cfg(windows)]
        let working_set_sizes = if self.allow_working_set_resize {
            Some(win_working_set::replace_set_size(self.mem_usize_count)?)
        } else {
            None
        };

        let lockguard = self.memory_resize_and_lock().ok();
        let mlocked = lockguard.is_some();

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

            let test_result = if time_left == 0 {
                Err(memtest::MemtestError::Timeout)
            } else if self.allow_multithread {
                struct ThreadBasePtr(*mut usize);
                unsafe impl Send for ThreadBasePtr {}

                let num_threads = num_cpus::get();
                let mut handles = vec![];
                let thread_memcount = self.mem_usize_count / num_threads;
                for i in 0..num_threads {
                    let thread_base_ptr = ThreadBasePtr(self.base_ptr.add(thread_memcount * i));
                    let handle = std::thread::spawn(move || {
                        let thread_base_ptr = thread_base_ptr;
                        test(thread_base_ptr.0, thread_memcount, time_left)
                    });
                    handles.push(handle);
                }

                handles
                    .into_iter()
                    .map(|handle| handle.join().unwrap_or(Err(MemtestError::Unknown)))
                    .fold(Ok(MemtestOutcome::Pass), |acc, result| {
                        use MemtestError::*;
                        use MemtestOutcome::*;
                        match (acc, result) {
                            (Err(Unknown), _) | (_, Err(Unknown)) => Err(Unknown),
                            (Err(Timeout), _) | (_, Err(Timeout)) => Err(Timeout),
                            (Ok(Fail(addr1, addr2)), _) | (_, Ok(Fail(addr1, addr2))) => {
                                Ok(Fail(addr1, addr2))
                            }
                            _ => Ok(Pass),
                        }
                    })
            } else {
                test(self.base_ptr, self.mem_usize_count, time_left)
            };

            reports.push(MemtestReport::new(*test_type, test_result));
        }

        // Unlock memory
        drop(lockguard);

        #[cfg(windows)]
        if let Some((min_set_size, max_set_size)) = working_set_sizes {
            win_working_set::restore_set_size(min_set_size, max_set_size)?;
        }

        Ok(MemtestReportList {
            tested_usize_count: self.mem_usize_count,
            mlocked,
            reports,
        })
    }

    fn memory_resize_and_lock(&mut self) -> Result<region::LockGuard, ()> {
        // TODO: get PAGE_SIZE from OS
        const PAGE_SIZE: usize = 4096;
        const WIN_OUTOFMEM_CODE: usize = 1453;
        let mut memsize = self.mem_usize_count * size_of::<usize>();
        loop {
            match region::lock(self.base_ptr, memsize) {
                Ok(lockguard) => {
                    self.mem_usize_count = memsize / size_of::<usize>();
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
                    match memsize.checked_sub(PAGE_SIZE) {
                        Some(new_memsize) => memsize = new_memsize,
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
        mem_usize_count: usize,
    ) -> Result<(usize, usize), MemtesterError> {
        let memsize = mem_usize_count * size_of::<usize>();
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
