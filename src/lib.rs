use {
    memtest::{MemtestError, MemtestOutcome, MemtestType},
    prelude::*,
    rand::{seq::SliceRandom, thread_rng},
    std::{
        fmt,
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

/// An internal structure to ensure the test timeouts in a given time frame
#[derive(Clone, Debug)]
struct TimeoutChecker {
    deadline: Instant,

    // TODO: Consider wrapping the members below in an additional struct
    test_start_time: Instant,
    expected_iter: u64,
    completed_iter: u64,
    checkpoint: u64,
    num_checks_completed: u128,
    checking_interval: Duration,
}

impl Memtester {
    // TODO: Memtester without given base_ptr, ie. take care of memory allocation as well
    // TODO: More configuration parameters:
    //       early termination? terminate per test vs all test?

    // NOTE: size of memory may be decremented for mlock
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
    pub fn run(self, memory: &mut [usize]) -> anyhow::Result<MemtestReportList> {
        let mut timeout_checker = TimeoutChecker::new(Instant::now(), self.timeout);

        // TODO: the linux memtester aligns base_ptr before mlock to avoid locking an extra page
        //       By default mlock rounds base_ptr down to nearest page boundary
        //       Not sure which is desirable

        #[cfg(windows)]
        let working_set_sizes = if self.allow_working_set_resize {
            Some(
                windows::replace_set_size(size_of_val(memory))
                    .context("Failed to replace process working set size")?,
            )
        } else {
            None
        };

        let (memory, mlocked) = match memory_resize_and_lock(memory, self.allow_mem_resize) {
            Ok(resized_memory) => (resized_memory, true),
            Err(e) => {
                warn!("Due to error, memory test will be run without memory locked: {e:?}");
                (memory, false)
            }
        };

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

            let test_result = if self.allow_multithread {
                std::thread::scope(|scope| {
                    let num_threads = num_cpus::get();
                    let chunk_size = memory.len() / num_threads;

                    // TODO: Take care of edge case where chunk_size is larger than memory count
                    let mut handles = vec![];
                    for chunk in memory.chunks_exact_mut(chunk_size) {
                        let mut timeout_checker = timeout_checker.clone();
                        let handle = scope.spawn(move || unsafe {
                            test(chunk.as_mut_ptr(), chunk.len(), &mut timeout_checker)
                        });
                        handles.push(handle);
                    }

                    handles
                        .into_iter()
                        .map(|handle| {
                            handle
                                .join()
                                .unwrap_or(Err(MemtestError::Other(anyhow!("Thread panicked"))))
                        })
                        .fold(Ok(MemtestOutcome::Pass), |acc, result| {
                            use {MemtestError::*, MemtestOutcome::*};
                            match (acc, result) {
                                (Err(Other(e)), _) | (_, Err(Other(e))) => Err(Other(e)),
                                (Err(Timeout), _) | (_, Err(Timeout)) => Err(Timeout),
                                (Ok(Fail(addr1, addr2)), _) | (_, Ok(Fail(addr1, addr2))) => {
                                    Ok(Fail(addr1, addr2))
                                }
                                _ => Ok(Pass),
                            }
                        })
                })
            } else {
                unsafe { test(memory.as_mut_ptr(), memory.len(), &mut timeout_checker) }
            };

            reports.push(MemtestReport::new(*test_type, test_result));
        }

        if let Err(e) = memory_unlock(memory) {
            warn!("Failed to unlock memory: {e:?}");
        }

        #[cfg(windows)]
        if let Some((min_set_size, max_set_size)) = working_set_sizes {
            if let Err(e) = windows::restore_set_size(min_set_size, max_set_size) {
                // TODO: Is there a need to tell the caller that set size is changed apart
                //       in addition to logging
                warn!("Failed to restore working size: {e:?}");
            }
        }

        Ok(MemtestReportList {
            tested_usize_count: size_of_val(memory),
            mlocked,
            reports,
        })
    }
}

impl fmt::Display for MemtestReportList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tested_memsize = {}\n", self.tested_usize_count)?;
        write!(f, "mlocked = {}\n", self.mlocked)?;
        for report in &self.reports {
            write!(
                f,
                "{:<30} {}",
                format!("Ran {:?}", report.test_type),
                format!("Outcome is {:?}\n", report.outcome)
            )?;
        }
        Ok(())
    }
}

impl MemtestReport {
    fn new(test_type: MemtestType, outcome: Result<MemtestOutcome, MemtestError>) -> MemtestReport {
        MemtestReport { test_type, outcome }
    }
}

impl TimeoutChecker {
    fn new(start_time: Instant, timeout: Duration) -> TimeoutChecker {
        TimeoutChecker {
            deadline: start_time + timeout,

            // TODO: Better placeholder values? all fields below are reset in `init()`
            test_start_time: Instant::now(),
            expected_iter: 0,
            completed_iter: 0,
            checkpoint: 1,
            num_checks_completed: 0,
            // TODO: Choice of starting interval is arbitrary for now.
            checking_interval: Duration::from_nanos(1000),
        }
    }

    /// This function should be called in the beginning of a memtest.
    /// It initializes struct members and set `expected_iter`.
    fn init(&mut self, expected_iter: u64) {
        self.test_start_time = Instant::now();
        self.expected_iter = expected_iter;
        self.completed_iter = 0;
        self.checkpoint = 1;
        self.num_checks_completed = 0;
        // TODO: Choice of starting interval is arbitrary for now.
        self.checking_interval = Duration::from_nanos(1000);
    }

    // TODO: TimeoutChecker is quite intertwined with MemtestError, might need decoupling later
    /// This function should be called in every iteration of a memtest
    ///
    /// To minimize overhead, the time is only checked at specific checkpoints.
    /// The algorithm makes a prediction of how many iterations will be done during `checking_interval`
    /// and sets the checkpoint based on the prediction.
    ///
    /// The algorithm also determines whether it is likely that the test will be completed
    /// based on `work_progress` and `time_progress`.
    /// If it is likely that the test will be completed, `checking_interval_ns` is scaled up to be more
    /// lenient and reduce overhead.
    fn check(&mut self) -> Result<(), MemtestError> {
        // TODO: Not sure how to remove use of `as` to get 2 u64 divide into an f64
        let work_progress = self.completed_iter as f64 / self.expected_iter as f64;
        // TODO: This current method of displaying progress is quite limited, especially for multithreading
        if self.completed_iter % (self.expected_iter / 100) == 0 {
            trace!("Progress: {:.0}%", work_progress * 100.0);
        }

        if self.completed_iter < self.checkpoint {
            self.completed_iter += 1;
            return Ok(());
        }

        let curr_time = Instant::now();
        if curr_time >= self.deadline {
            return Err(MemtestError::Timeout);
        }

        let test_elapsed = curr_time - self.test_start_time;
        let time_progress = test_elapsed.div_duration_f64(self.deadline - self.test_start_time);
        // TODO: Consider having a max for `checking_interval` to have a reasonable timeout guarantee
        if work_progress > time_progress {
            self.checking_interval = self.checking_interval.saturating_mul(2);
        }

        let avg_iter_duration = test_elapsed.div_f64(self.completed_iter as f64);
        let iter_per_interval = self.checking_interval.div_duration_f64(avg_iter_duration) as u64;
        self.checkpoint = self.completed_iter + iter_per_interval;

        self.num_checks_completed += 1;
        self.completed_iter += 1;
        Ok(())
    }
}

fn memory_resize_and_lock<'a>(
    memory: &'a mut [usize],
    allow_mem_resize: bool,
) -> anyhow::Result<&'a mut [usize]> {
    #[cfg(windows)]
    {
        crate::windows::memory_resize_and_lock(memory, allow_mem_resize)
    }

    #[cfg(unix)]
    {
        crate::unix::memory_resize_and_lock(memory, allow_mem_resize)
    }
}

fn memory_unlock<'a>(memory: &'a mut [usize]) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        crate::windows::memory_unlock(memory)
    }

    #[cfg(unix)]
    {
        crate::unix::memory_unlock(memory)
    }
}

#[cfg(windows)]
mod windows {
    use {
        crate::prelude::*,
        std::mem::MaybeUninit,
        windows::Win32::{
            Foundation::ERROR_WORKING_SET_QUOTA,
            System::{
                Memory::{VirtualLock, VirtualUnlock},
                SystemInformation::{GetNativeSystemInfo, SYSTEM_INFO},
                Threading::{
                    GetCurrentProcess, GetProcessWorkingSetSize, SetProcessWorkingSetSize,
                },
            },
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

    // TODO: Rethink options for handling mlock failure
    // The linux memtester always tries to mlock,
    // If mlock returns with ENOMEM or EAGAIN, it resizes memory.
    // If mlock returns with EPERM or unknown error, it moves forward to tests with unlocked memory.
    // It is unclear whether testing unlocked memory is something useful
    // TODO: Rewrite this function to better handle errors, bail & resize
    // TODO: Check for timeout, decrementing memory size can take non trivial time
    pub(super) fn memory_resize_and_lock<'a>(
        mut memory: &'a mut [usize],
        allow_mem_resize: bool,
    ) -> anyhow::Result<&'a mut [usize]> {
        let page_size = unsafe {
            let mut sysinfo: MaybeUninit<SYSTEM_INFO> = MaybeUninit::uninit();
            GetNativeSystemInfo(sysinfo.as_mut_ptr());
            usize::try_from(sysinfo.assume_init().dwPageSize)
                .context("Failed to convert page size to usize")?
        };

        loop {
            unsafe {
                match VirtualLock(memory.as_mut_ptr().cast(), size_of_val(memory)) {
                    Ok(()) => {
                        info!("Successfully locked {}MB", size_of_val(memory));
                        return Ok(memory);
                    }
                    Err(e) if e.code() == ERROR_WORKING_SET_QUOTA.to_hresult() => {
                        if allow_mem_resize {
                            match size_of_val(memory).checked_sub(page_size) {
                                    Some(new_memsize) => {
                                        warn!("Decremented memory size to {new_memsize}MB, retry memory locking");
                                        memory = &mut memory[0..new_memsize / size_of::<usize>()];
                                    }
                                    None => bail!("Failed to lock any memory, memory size has been decremented to 0"),  
                                }
                        } else {
                            bail!("VirtualLock failed to lock requested memory size: {e:?}")
                        }
                    }
                    Err(e) => {
                        return Err(anyhow!(e).context("VirtualLock failed to lock memory"));
                    }
                }
            }
        }
    }

    pub(super) fn memory_unlock<'a>(memory: &'a mut [usize]) -> anyhow::Result<()> {
        unsafe {
            VirtualUnlock(memory.as_mut_ptr().cast(), size_of_val(memory))
                .context("VirtualUnlock failed")
        }
    }
}

#[cfg(unix)]
mod unix {
    use {
        crate::prelude::*,
        nix::{
            errno::Errno,
            sys::mman::{mlock, munlock},
            unistd::{sysconf, SysconfVar},
        },
        std::ptr::NonNull,
    };

    // TODO: Rethink options for handling mlock failure
    // The linux memtester always tries to mlock,
    // If mlock returns with ENOMEM or EAGAIN, it resizes memory.
    // If mlock returns with EPERM or unknown error, it moves forward to tests with unlocked memory.
    // It is unclear whether testing unlocked memory is something useful
    // TODO: Rewrite this function to better handle errors, bail & resize
    // TODO: Check for timeout, decrementing memory size can take non trivial time
    // TODO: Resize to RLIMIT_MEMLOCK instead of decrementing (note: memory might not be page aligned so locking the limit can still fail);
    pub(super) fn memory_resize_and_lock<'a>(
        mut memory: &'a mut [usize],
        allow_mem_resize: bool,
    ) -> anyhow::Result<&'a mut [usize]> {
        let Ok(Some(page_size)) = sysconf(SysconfVar::PAGE_SIZE) else {
            bail!("Failed to get page size");
        };
        let page_size =
            usize::try_from(page_size).context("Failed to convert page size to usize")?;

        loop {
            unsafe {
                match mlock(
                    NonNull::new(memory.as_mut_ptr().cast()).unwrap(),
                    size_of_val(memory),
                ) {
                    Ok(()) => {
                        info!("Successfully locked {}MB", size_of_val(memory));
                        return Ok(memory);
                    }
                    Err(Errno::ENOMEM) => {
                        if allow_mem_resize {
                            match size_of_val(memory).checked_sub(page_size) {
                                    Some(new_memsize) => {
                                        warn!("Decremented memory size to {new_memsize}MB, retry memory locking");
                                        memory = &mut memory[0..new_memsize / size_of::<usize>()];
                                    }
                                    None => bail!("Failed to lock any memory, memory size has been decremented to 0"),
                                }
                        } else {
                            bail!(
                                "mlock failed to lock requested memory size: {:?}",
                                Errno::ENOMEM
                            )
                        }
                    }
                    Err(e) => {
                        return Err(anyhow!(e).context("mlock failed to lock memory"));
                    }
                }
            }
        }
    }

    pub(super) fn memory_unlock<'a>(memory: &'a mut [usize]) -> anyhow::Result<()> {
        unsafe {
            munlock(
                NonNull::new(memory.as_mut_ptr().cast()).unwrap(),
                size_of_val(memory),
            )
            .context("munlock failed")
        }
    }
}
