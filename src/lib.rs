use {
    memtest::{MemtestError, MemtestOutcome, MemtestType},
    prelude::*,
    rand::{seq::SliceRandom, thread_rng},
    std::{
        io::ErrorKind,
        time::{Duration, Instant},
    },
};

mod memtest;
mod prelude;

#[derive(Debug)]
pub struct Memtester {
    timeout: Duration,
    allow_working_set_resize: bool,
    allow_mem_resize: bool,
    allow_multithread: bool,
    test_types: Vec<MemtestType>,
}

#[derive(Debug)]
pub struct MemtesterArgs {
    pub timeout: Duration,
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
pub struct MemtestReport {
    pub test_type: MemtestType,
    pub outcome: Result<MemtestOutcome, MemtestError>,
}

impl Memtester {
    // TODO: Memtester without given base_ptr, ie. take care of memory allocation as well
    // TODO: More configuration parameters:
    //       early termination? terminate per test vs all test?

    // NOTE: `mem_usize_count` may be decremented for mlock
    /// Create a Memtester containing all test types in random order
    pub fn all_tests_random_order(args: MemtesterArgs) -> Memtester {
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

        Self::from_test_types(args, test_types)
    }

    /// Create a Memtester with specified test types
    pub fn from_test_types(args: MemtesterArgs, test_types: Vec<MemtestType>) -> Memtester {
        Memtester {
            timeout: args.timeout,
            allow_working_set_resize: args.allow_working_set_resize,
            allow_mem_resize: args.allow_mem_resize,
            allow_multithread: args.allow_multithread,
            test_types,
        }
    }

    /// Consume the Memtester and run the tests
    pub unsafe fn run(mut self, memory: &mut [usize]) -> anyhow::Result<MemtestReportList> {
        let start_time = Instant::now();

        // TODO: the linux memtester aligns base_ptr before mlock to avoid locking an extra page
        //       By default mlock rounds base_ptr down to nearest page boundary
        //       Not sure which is desirable

        #[cfg(windows)]
        let working_set_sizes = if self.allow_working_set_resize {
            Some(
                win_working_set::replace_set_size(size_of_val(memory))
                    .context("Failed to replace process working set size")?,
            )
        } else {
            None
        };

        let lock_guard = match self.memory_resize_and_lock(memory) {
            Ok(guard) => Some(guard),
            Err(e) => {
                warn!("Due to error, memory test will be run without region locked: {e:?}");
                None
            }
        };
        let mlocked = lock_guard.is_some();

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
            let time_left = self.timeout.saturating_sub(start_time.elapsed());

            let test_result = if time_left.is_zero() {
                Err(memtest::MemtestError::Timeout)
            } else if self.allow_multithread {
                std::thread::scope(|scope| {
                    let num_threads = num_cpus::get();
                    let chunk_size = memory.len() / num_threads;

                    let mut handles = vec![];
                    for chunk in memory.chunks_exact_mut(chunk_size) {
                        let handle =
                            scope.spawn(move || test(chunk.as_mut_ptr(), chunk.len(), time_left));
                        handles.push(handle);
                    }

                    handles
                        .into_iter()
                        .map(|handle| handle.join().unwrap_or(Err(MemtestError::Unknown)))
                        .fold(Ok(MemtestOutcome::Pass), |acc, result| {
                            use {MemtestError::*, MemtestOutcome::*};
                            match (acc, result) {
                                (Err(Unknown), _) | (_, Err(Unknown)) => Err(Unknown),
                                (Err(Timeout), _) | (_, Err(Timeout)) => Err(Timeout),
                                (Ok(Fail(addr1, addr2)), _) | (_, Ok(Fail(addr1, addr2))) => {
                                    Ok(Fail(addr1, addr2))
                                }
                                _ => Ok(Pass),
                            }
                        })
                })
            } else {
                test(memory.as_mut_ptr(), memory.len(), time_left)
            };

            reports.push(MemtestReport::new(*test_type, test_result));
        }

        // Unlock memory
        drop(lock_guard);

        #[cfg(windows)]
        if let Some((min_set_size, max_set_size)) = working_set_sizes {
            if let Err(e) = win_working_set::restore_set_size(min_set_size, max_set_size) {
                // TODO: Is there a need to tell the caller that set size is changed apart
                //       apart from logging
                warn!("Failed to restore working size: {e:?}");
            }
        }

        Ok(MemtestReportList {
            tested_usize_count: size_of_val(memory),
            mlocked,
            reports,
        })
    }

    // TODO: Rewrite this function to better handle errors, bail & resize
    // TODO: replace `region`
    fn memory_resize_and_lock(
        &mut self,
        mut memory: &mut [usize],
    ) -> anyhow::Result<region::LockGuard> {
        const WIN_OUTOFMEM_CODE: usize = 1453;
        let page_size: usize = region::page::size();

        loop {
            match region::lock(memory.as_mut_ptr(), size_of_val(memory)) {
                Ok(lockguard) => {
                    return Ok(lockguard);
                }
                // TODO: macOS error?
                Err(region::Error::SystemCall(err))
                    if (matches!(err.kind(), ErrorKind::OutOfMemory)
                        || err
                            .raw_os_error()
                            .is_some_and(|e| e as usize == WIN_OUTOFMEM_CODE)) =>
                {
                    if self.allow_mem_resize {
                        match size_of_val(memory).checked_sub(page_size) {
                            Some(new_memsize) => memory = &mut memory[0..new_memsize],
                            None => bail!("Failed to lock any memory"),
                        }
                    } else {
                        bail!("Failed to lock requested amount of memory due to {err:?}")
                    }
                }
                Err(e) => return Err(anyhow!(e).context("Failed to lock memory")),
            }
        }
    }
}

impl MemtestReport {
    fn new(test_type: MemtestType, outcome: Result<MemtestOutcome, MemtestError>) -> MemtestReport {
        MemtestReport { test_type, outcome }
    }
}

#[cfg(windows)]
mod win_working_set {
    use {
        crate::prelude::*,
        windows::Win32::System::Threading::{
            GetCurrentProcess, GetProcessWorkingSetSize, SetProcessWorkingSetSize,
        },
    };

    pub(super) fn replace_set_size(memsize: usize) -> anyhow::Result<(usize, usize)> {
        let (mut min_set_size, mut max_set_size) = (0, 0);
        unsafe {
            GetProcessWorkingSetSize(GetCurrentProcess(), &mut min_set_size, &mut max_set_size)
                .context("Failed to get process working set")?;
            // TODO: Not sure what the best choice of min and max should be
            SetProcessWorkingSetSize(
                GetCurrentProcess(),
                memsize.saturating_mul(2),
                memsize.saturating_mul(4),
            )
            .context("Failed to set process working set")?;
        }
        Ok((min_set_size, max_set_size))
    }

    pub(super) fn restore_set_size(min_set_size: usize, max_set_size: usize) -> anyhow::Result<()> {
        unsafe {
            SetProcessWorkingSetSize(GetCurrentProcess(), min_set_size, max_set_size)
                .context("Failed to set process working set")
        }
    }
}
