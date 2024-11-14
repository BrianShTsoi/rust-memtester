#[cfg(unix)]
use unix::{memory_resize_and_lock, memory_unlock};
#[cfg(windows)]
use windows::{memory_resize_and_lock, memory_unlock, replace_set_size};
use {
    memtest::{MemtestError, MemtestKind, MemtestOutcome},
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
    test_types: Vec<MemtestKind>,
    timeout: Duration,
    require_memlock: bool,
    #[allow(dead_code)]
    allow_working_set_resize: bool,
    allow_mem_resize: bool,
    allow_multithread: bool,
    allow_early_termination: bool,
}

// TODO: Replace MemtesterArgs with a Builder struct implementing fluent interface
/// A set of arguments that define the behavior of Memtester
#[derive(Debug)]
pub struct MemtesterArgs {
    /// How long should Memtester run the test suite before timing out
    pub timeout: Duration,
    /// Whether memory will be locked before testing
    /// If memory locking failed but is required, Memtester returns with error
    pub require_memlock: bool,
    /// Whether the process working set can be resized to accomodate memory locking
    /// This argument is only meaninful for Windows
    pub allow_working_set_resize: bool,
    /// Whether the requested memory size of testing can be reduced to accomodate memory locking
    /// This argument is only meaninful is memlock is required
    pub allow_mem_resize: bool,
    /// Whether mulithreading is enabled
    pub allow_multithread: bool,
    /// Whether Memtester returns immediately if a test fails or continues until all tests are run
    pub allow_early_termination: bool,
}

#[derive(Debug)]
pub struct MemtestReportList {
    pub tested_usize_count: usize,
    pub mlocked: bool,
    pub reports: Vec<MemtestReport>,
}

#[derive(Debug)]
pub struct MemtestReport {
    pub test_type: MemtestKind,
    pub outcome: Result<MemtestOutcome, MemtestError>,
}

/// An struct to ensure the test timeouts in a given duration
#[derive(Clone, Debug)]
struct TimeoutChecker {
    deadline: Instant,
    test_start_time: Instant,
    expected_iter: u64,
    completed_iter: u64,
    checkpoint: u64,
    // TODO: `num_checks_completed` is purely for debugging & can be removed
    num_checks_completed: u128,
    last_progress_fraction: f32,
}

impl Memtester {
    /// Create a Memtester containing all test types in random order
    pub fn all_tests_random_order(args: MemtesterArgs) -> Memtester {
        let mut test_types = vec![
            MemtestKind::OwnAddress,
            MemtestKind::RandomVal,
            MemtestKind::Xor,
            MemtestKind::Sub,
            MemtestKind::Mul,
            MemtestKind::Div,
            MemtestKind::Or,
            MemtestKind::And,
            MemtestKind::SeqInc,
            MemtestKind::SolidBits,
            MemtestKind::Checkerboard,
            MemtestKind::BlockSeq,
        ];
        test_types.shuffle(&mut thread_rng());

        Self::from_test_types(args, test_types)
    }

    /// Create a Memtester with specified test types
    pub fn from_test_types(args: MemtesterArgs, test_types: Vec<MemtestKind>) -> Memtester {
        Memtester {
            test_types,
            timeout: args.timeout,
            require_memlock: args.require_memlock,
            allow_working_set_resize: args.allow_working_set_resize,
            allow_mem_resize: args.allow_mem_resize,
            allow_multithread: args.allow_multithread,
            allow_early_termination: args.allow_early_termination,
        }
    }

    /// Run the tests, possibly after locking the memory
    pub fn run(&self, memory: &mut [usize]) -> anyhow::Result<MemtestReportList> {
        // TODO: Should have a minimum memory length so that we don't UB when `memory.len()` is too small
        let deadline = Instant::now() + self.timeout;

        // TODO: the linux memtester aligns base_ptr before mlock to avoid locking an extra page
        //       By default mlock rounds base_ptr down to nearest page boundary
        //       Not sure which is desirable

        if self.require_memlock {
            #[cfg(windows)]
            let _working_set_resize_guard = if self.allow_working_set_resize {
                Some(
                    replace_set_size(size_of_val(memory))
                        .context("failed to replace process working set size")?,
                )
            } else {
                None
            };

            let (memory, mlocked) = match memory_resize_and_lock(memory, self.allow_mem_resize) {
                Ok(resized_memory) => (resized_memory, true),
                Err(e) => {
                    // TODO: Returning without restoring set size?
                    bail!(e.context("Failed memory locking when it is required"));
                }
            };

            let reports = self.run_tests(memory, deadline);

            if let Err(e) = memory_unlock(memory) {
                warn!("Failed to unlock memory: {e:?}");
            }

            Ok(MemtestReportList {
                tested_usize_count: size_of_val(memory),
                mlocked,
                reports,
            })
        } else {
            Ok(MemtestReportList {
                tested_usize_count: size_of_val(memory),
                mlocked: false,
                reports: self.run_tests(memory, deadline),
            })
        }
    }

    /// Run tests
    fn run_tests(&self, memory: &mut [usize], deadline: Instant) -> Vec<MemtestReport> {
        let mut reports = Vec::new();
        for test_type in &self.test_types {
            let test = match test_type {
                MemtestKind::OwnAddress => memtest::test_own_address,
                MemtestKind::RandomVal => memtest::test_random_val,
                MemtestKind::Xor => memtest::test_xor,
                MemtestKind::Sub => memtest::test_sub,
                MemtestKind::Mul => memtest::test_mul,
                MemtestKind::Div => memtest::test_div,
                MemtestKind::Or => memtest::test_or,
                MemtestKind::And => memtest::test_and,
                MemtestKind::SeqInc => memtest::test_seq_inc,
                MemtestKind::SolidBits => memtest::test_solid_bits,
                MemtestKind::Checkerboard => memtest::test_checkerboard,
                MemtestKind::BlockSeq => memtest::test_block_seq,
            };

            let test_result = if self.allow_multithread {
                std::thread::scope(|scope| {
                    let num_threads = std::cmp::min(num_cpus::get(), memory.len());
                    let chunk_size = memory.len() / num_threads;

                    let mut handles = vec![];
                    for chunk in memory.chunks_exact_mut(chunk_size) {
                        let handle = scope.spawn(|| unsafe {
                            test(
                                chunk.as_mut_ptr(),
                                chunk.len(),
                                &mut TimeoutChecker::new(deadline),
                            )
                        });
                        handles.push(handle);
                    }

                    #[allow(clippy::manual_try_fold)]
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
                                (Ok(Fail(f)), _) | (_, Ok(Fail(f))) => Ok(Fail(f)),
                                _ => Ok(Pass),
                            }
                        })
                })
            } else {
                unsafe {
                    test(
                        memory.as_mut_ptr(),
                        memory.len(),
                        &mut TimeoutChecker::new(deadline),
                    )
                }
            };

            if matches!(test_result, Ok(MemtestOutcome::Fail(_))) && self.allow_early_termination {
                reports.push(MemtestReport::new(*test_type, test_result));
                warn!("Memtest failed, terminating early");
                break;
            }
            reports.push(MemtestReport::new(*test_type, test_result));
        }

        reports
    }
}

impl fmt::Display for MemtestReportList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "tested_memsize = {}", self.tested_usize_count)?;
        writeln!(f, "mlocked = {}", self.mlocked)?;
        for report in &self.reports {
            let outcome = match &report.outcome {
                Ok(outcome) => format!("{}", outcome),
                Err(e) => format!("{}", e),
            };
            writeln!(
                f,
                "{:<30} {}",
                format!("Ran Test: {:?}", report.test_type),
                outcome
            )?;
        }
        Ok(())
    }
}

impl MemtestReportList {
    pub fn iter(&self) -> std::slice::Iter<'_, MemtestReport> {
        self.reports.iter()
    }

    /// Returns true if all tests were run successfully and all tests passed
    pub fn all_pass(&self) -> bool {
        self.iter()
            .all(|report| matches!(report.outcome, Ok(MemtestOutcome::Pass)))
    }
}

impl MemtestReport {
    fn new(test_type: MemtestKind, outcome: Result<MemtestOutcome, MemtestError>) -> MemtestReport {
        MemtestReport { test_type, outcome }
    }
}

impl TimeoutChecker {
    fn new(deadline: Instant) -> TimeoutChecker {
        TimeoutChecker {
            deadline,
            test_start_time: Instant::now(), // placeholder, gets reset in `init()`
            expected_iter: 0,                // placeholder, gets reset in `init()`
            completed_iter: 0,
            checkpoint: 1, // placeholder, gets reset in `init()`
            num_checks_completed: 0,
            // TODO: This is an arbitrary choice of starting interval
            // checking_interval: Duration::from_nanos(1000),
            last_progress_fraction: 0.0,
        }
    }

    /// This function should be called in the beginning of a memtest.
    /// It initializes `checkpoint`, `test_start_time` and `expected_iter`
    fn init(&mut self, expected_iter: u64) {
        const FIRST_CHECKPOINT: u64 = 8;
        self.test_start_time = Instant::now();
        self.expected_iter = expected_iter;

        // The first checkpoint is set to 8 if `expected_iter` is sufficiently large
        // This means the checker waits for 8 iterations before calling `check_time()` in order to
        // have a more accurate sample of duration per iteration for determining new checkpoint
        self.checkpoint = if expected_iter > FIRST_CHECKPOINT {
            FIRST_CHECKPOINT
        } else {
            1
        };
    }

    /// This function should be called in every iteration of a memtest
    ///
    /// To reduce overhead, the function only checks for timeout at specific checkpoints, and
    /// early returns otherwise.
    ///
    // It is important to ensure that the "early return" hot path is inlined. This results in a
    // 100% improvement in performance.
    #[inline(always)]
    fn check(&mut self) -> Result<(), MemtestError> {
        if self.completed_iter < self.checkpoint {
            self.completed_iter += 1;
            return Ok(());
        }

        let progress_fraction = self.completed_iter as f32 / self.expected_iter as f32;
        if progress_fraction - self.last_progress_fraction >= 0.01 {
            trace!("Progress: {:.0}%", progress_fraction * 100.0);
            self.last_progress_fraction = progress_fraction;
        }

        self.check_time()
    }

    /// Checkpoints are either initialized or scheduled at the previous checkpoint. The algorithm
    /// calculates the remaining time before the deadline and schedules the next check at 75% of
    /// that interval, and estimate the number of iterations to get there.
    fn check_time(&mut self) -> Result<(), MemtestError> {
        const DEADLINE_CHECK_RATIO: f64 = 0.75;

        let current_time = Instant::now();
        if current_time >= self.deadline {
            return Err(MemtestError::Timeout);
        }

        let duration_until_next_checkpoint = {
            let duration_until_deadline = self.deadline - current_time;
            duration_until_deadline.mul_f64(DEADLINE_CHECK_RATIO)
        };

        let avg_iter_duration = {
            let test_elapsed = current_time - self.test_start_time;
            test_elapsed.div_f64(self.completed_iter as f64)
        };

        let iter_until_next_checkpoint = {
            let x = duration_until_next_checkpoint.div_duration_f64(avg_iter_duration) as u64;
            u64::max(x, 1)
        };

        self.checkpoint += iter_until_next_checkpoint;

        self.num_checks_completed += 1;
        self.completed_iter += 1;
        Ok(())
    }
}

// TODO: Rethink options for handling mlock failure
//       The linux memtester always tries to mlock,
//       If mlock returns with ENOMEM or EAGAIN, it resizes memory.
//       If mlock returns with EPERM or unknown error, it moves forward to tests with unlocked memory.
//       It is unclear whether testing unlocked memory is something useful
// TODO: Check for timeout, decrementing memory size can take non trivial time

#[cfg(windows)]
mod windows {
    use {
        crate::prelude::*,
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

    #[derive(Debug)]
    pub(super) struct WorkingSetResizeGuard {
        min_set_size: usize,
        max_set_size: usize,
    }

    // TODO: We don't really need to replace the set size if if it is much larger than memsize?
    pub(super) fn replace_set_size(memsize: usize) -> anyhow::Result<WorkingSetResizeGuard> {
        let (mut min_set_size, mut max_set_size) = (0, 0);
        unsafe {
            GetProcessWorkingSetSize(GetCurrentProcess(), &mut min_set_size, &mut max_set_size)
                .context("failed to get process working set")?;
            // TODO: Not sure what the best choice of min and max should be
            SetProcessWorkingSetSize(
                GetCurrentProcess(),
                memsize.saturating_mul(2),
                memsize.saturating_mul(4),
            )
            .context("failed to set process working set")?;
        }
        Ok(WorkingSetResizeGuard {
            min_set_size,
            max_set_size,
        })
    }

    impl Drop for WorkingSetResizeGuard {
        fn drop(&mut self) {
            unsafe {
                if let Err(e) = SetProcessWorkingSetSize(
                    GetCurrentProcess(),
                    self.min_set_size,
                    self.max_set_size,
                ) {
                    warn!("Failed to restore process working set: {e}");
                }
            }
        }
    }

    // TODO: Resize according to min set size instead of decrementing
    pub(super) fn memory_resize_and_lock(
        mut memory: &mut [usize],
        allow_mem_resize: bool,
    ) -> anyhow::Result<&mut [usize]> {
        let page_size = usize::try_from(unsafe {
            let mut sysinfo: SYSTEM_INFO = std::mem::zeroed();
            GetNativeSystemInfo(&mut sysinfo);
            sysinfo.dwPageSize
        })
        .unwrap();
        let usize_per_page = page_size / std::mem::size_of::<usize>();

        loop {
            let res = unsafe { VirtualLock(memory.as_mut_ptr().cast(), size_of_val(memory)) };
            let Err(e) = res else {
                info!("Successfully locked {}MB", size_of_val(memory));
                return Ok(memory);
            };

            ensure!(
                e == ERROR_WORKING_SET_QUOTA.into(),
                anyhow!(e).context("VirtualLock failed to lock memroy")
            );
            ensure!(
                allow_mem_resize,
                anyhow!(e).context("VirtualLock failed to lock requested memory size")
            );

            let new_len = memory
                .len()
                .checked_sub(usize_per_page)
                .context("Failed to lock any memory, memory size has been decremented to 0")?;

            memory = &mut memory[0..new_len];
            warn!(
                "Decremented memory size to {}MB, retry memory locking",
                new_len * usize_per_page
            );
        }
    }

    pub(super) fn memory_unlock(memory: &mut [usize]) -> anyhow::Result<()> {
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
        libc::{mlock, munlock, sysconf, _SC_PAGESIZE},
        std::io::{Error, ErrorKind},
    };

    // TODO: Resize to RLIMIT_MEMLOCK instead of decrementing (note: memory might not be page aligned so locking the limit can still fail)
    pub(super) fn memory_resize_and_lock(
        mut memory: &mut [usize],
        allow_mem_resize: bool,
    ) -> anyhow::Result<&mut [usize]> {
        let page_size = usize::try_from(unsafe { sysconf(_SC_PAGESIZE) }).unwrap();
        let usize_per_page = page_size / std::mem::size_of::<usize>();

        loop {
            if unsafe { mlock(memory.as_mut_ptr().cast(), size_of_val(memory)) } == 0 {
                info!("Successfully locked {}MB", size_of_val(memory));
                return Ok(memory);
            }

            let e = Error::last_os_error();
            ensure!(
                e.kind() == ErrorKind::OutOfMemory,
                anyhow!(e).context("mlock failed")
            );
            ensure!(
                allow_mem_resize,
                anyhow!(e).context("mlock failed to lock requested memory size")
            );

            let new_len = memory
                .len()
                .checked_sub(usize_per_page)
                .context("Failed to lock any memory, memory size has been decremented to 0")?;
            memory = &mut memory[0..new_len];
            warn!(
                "Decremented memory size to {}MB, retry memory locking",
                new_len * usize_per_page
            );
        }
    }

    pub(super) fn memory_unlock(memory: &mut [usize]) -> anyhow::Result<()> {
        unsafe {
            match munlock(memory.as_mut_ptr().cast(), size_of_val(memory)) {
                0 => Ok(()),
                _ => Err(Error::last_os_error()).context("munlock failed"),
            }
        }
    }
}
