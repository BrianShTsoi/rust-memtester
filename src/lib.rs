#[cfg(unix)]
use unix::memory_resize_and_lock;
#[cfg(windows)]
use windows::{memory_resize_and_lock, replace_set_size};
use {
    memtest::{MemtestError, MemtestKind, MemtestOutcome},
    prelude::*,
    rand::{seq::SliceRandom, thread_rng},
    std::{
        error::Error,
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
    mem_lock_mode: MemLockMode,
    #[allow(dead_code)]
    allow_working_set_resize: bool,
    allow_multithread: bool,
    allow_early_termination: bool,
}

// TODO: Replace MemtesterArgs with a Builder struct implementing fluent interface
/// A set of arguments that define the behavior of Memtester
#[derive(Debug)]
pub struct MemtesterArgs {
    /// How long should Memtester run the test suite before timing out
    pub timeout: Duration,
    /// Whether memory will be locked before testing and whether the requested memory size of
    /// testing can be reduced to accomodate memory locking
    /// If memory locking failed but is required, Memtester returns with error
    pub mem_lock_mode: MemLockMode,
    /// Whether the process working set can be resized to accomodate memory locking
    /// This argument is only meaninful for Windows
    pub allow_working_set_resize: bool,
    /// Whether mulithreading is enabled
    pub allow_multithread: bool,
    /// Whether Memtester returns immediately if a test fails or continues until all tests are run
    pub allow_early_termination: bool,
}

#[derive(Debug)]
pub enum MemtesterError {
    MemLockFailed(anyhow::Error),
    Other(anyhow::Error),
}

#[derive(Debug)]
pub struct MemtestReportList {
    pub tested_men_len: usize,
    pub mlocked: bool,
    pub reports: Vec<MemtestReport>,
}

#[derive(Debug)]
pub struct MemtestReport {
    pub test_type: MemtestKind,
    pub outcome: Result<MemtestOutcome, MemtestError>,
}

#[derive(Clone, Copy, Debug)]
pub enum MemLockMode {
    Resizable,
    FixedSize,
    Disabled,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseMemLockModeError;

/// The minimum memory length (in usize) for Memtester to run tests on
pub const MIN_MEMORY_LEN: usize = 512;

#[derive(Debug)]
struct MemLockGuard {
    base_ptr: *const (),
    mem_size: usize,
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
    pub fn all_tests_random_order(args: &MemtesterArgs) -> Memtester {
        let mut test_types = vec![
            // MemtestKind::OwnAddressBasic,
            MemtestKind::OwnAddressRepeat,
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
    pub fn from_test_types(args: &MemtesterArgs, test_types: Vec<MemtestKind>) -> Memtester {
        Memtester {
            test_types,
            timeout: args.timeout,
            mem_lock_mode: args.mem_lock_mode,
            allow_working_set_resize: args.allow_working_set_resize,
            allow_multithread: args.allow_multithread,
            allow_early_termination: args.allow_early_termination,
        }
    }

    /// Run the tests, possibly after locking the memory
    pub fn run(&self, memory: &mut [usize]) -> Result<MemtestReportList, MemtesterError> {
        if memory.len() < MIN_MEMORY_LEN {
            return Err(MemtesterError::Other(anyhow!("Insufficient memory length")));
        }

        let deadline = Instant::now() + self.timeout;

        // TODO: the linux memtester aligns base_ptr before mlock to avoid locking an extra page
        //       By default mlock rounds base_ptr down to nearest page boundary
        //       Not sure which is desirable
        match &self.mem_lock_mode {
            MemLockMode::Disabled => Ok(MemtestReportList {
                tested_men_len: memory.len(),
                mlocked: false,
                reports: self.run_tests(memory, deadline),
            }),

            mode => {
                #[cfg(windows)]
                let _working_set_resize_guard = if self.allow_working_set_resize {
                    Some(
                        replace_set_size(size_of_val(memory))
                            .context("failed to replace process working set size")?,
                    )
                } else {
                    None
                };

                let (memory, _mem_lock_guard) =
                    memory_resize_and_lock(memory, matches!(mode, MemLockMode::Resizable))
                        .map_err(MemtesterError::MemLockFailed)?;

                Ok(MemtestReportList {
                    tested_men_len: memory.len(),
                    mlocked: true,
                    reports: self.run_tests(memory, deadline),
                })
            }
        }
    }

    /// Run tests
    fn run_tests(&self, memory: &mut [usize], deadline: Instant) -> Vec<MemtestReport> {
        let mut reports = Vec::new();
        for test_type in &self.test_types {
            let test = match test_type {
                MemtestKind::OwnAddressBasic => memtest::test_own_address_basic,
                MemtestKind::OwnAddressRepeat => memtest::test_own_address_repeat,
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
                        let handle =
                            scope.spawn(|| test(chunk, &mut TimeoutChecker::new(deadline)));
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
                test(memory, &mut TimeoutChecker::new(deadline))
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

impl fmt::Display for MemtesterError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for MemtesterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            MemtesterError::MemLockFailed(err) | MemtesterError::Other(err) => Some(err.as_ref()),
        }
    }
}

impl From<anyhow::Error> for MemtesterError {
    fn from(err: anyhow::Error) -> MemtesterError {
        MemtesterError::Other(err)
    }
}

impl std::str::FromStr for MemLockMode {
    type Err = ParseMemLockModeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "resizable" => Ok(Self::Resizable),
            "fixedsize" => Ok(Self::FixedSize),
            "disabled" => Ok(Self::Disabled),
            _ => Err(ParseMemLockModeError),
        }
    }
}

impl fmt::Display for MemtestReportList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "tested_mem_len = {}", self.tested_men_len)?;
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

    /// Initialize the first checkpoint, test starting time and total expected iterations
    /// This function should be called in the beginning of a memtest.
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

    /// Check if the current iteration is a checkpoint. If so, check if timeout occurred
    ///
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
        crate::{prelude::*, MemLockGuard},
        std::cmp,
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
    pub struct WorkingSetResizeGuard {
        min_set_size: usize,
        max_set_size: usize,
    }

    pub(super) fn replace_set_size(memsize: usize) -> anyhow::Result<WorkingSetResizeGuard> {
        let (min_set_size, max_set_size) = get_set_size()?;
        unsafe {
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

    pub(super) fn memory_resize_and_lock(
        mut memory: &mut [usize],
        allow_mem_resize: bool,
    ) -> anyhow::Result<(&mut [usize], MemLockGuard)> {
        let page_size = usize::try_from(unsafe {
            let mut sysinfo: SYSTEM_INFO = std::mem::zeroed();
            GetNativeSystemInfo(&mut sysinfo);
            sysinfo.dwPageSize
        })
        .unwrap();
        let usize_per_page = page_size / std::mem::size_of::<usize>();

        loop {
            let base_ptr = memory.as_mut_ptr();
            let mem_size = size_of_val(memory);

            let res = unsafe { VirtualLock(base_ptr.cast(), mem_size) };
            let Err(e) = res else {
                info!("Successfully locked {}MB", mem_size);
                return Ok((
                    memory,
                    MemLockGuard {
                        base_ptr: base_ptr.cast(),
                        mem_size,
                    },
                ));
            };

            ensure!(
                e == ERROR_WORKING_SET_QUOTA.into(),
                anyhow!(e).context("VirtualLock failed to lock memroy")
            );
            ensure!(
                allow_mem_resize,
                anyhow!(e).context("VirtualLock failed to lock requested memory size")
            );

            // Set new_len to memory locking system limit if this is the first resize, otherwise
            // decrement by a page.
            // Note that locking with system limit can fail because the memory might not be page
            // aligned
            let new_len = cmp::min(
                get_set_size()?.0 / size_of::<usize>(),
                memory
                    .len()
                    .checked_sub(usize_per_page)
                    .context("Failed to lock any memory, memory size has been decremented to 0")?,
            );

            memory = &mut memory[0..new_len];
            warn!(
                "Decremented memory size to {}MB, retry memory locking",
                new_len * usize_per_page
            );
        }
    }

    impl Drop for MemLockGuard {
        fn drop(&mut self) {
            unsafe {
                if let Err(e) = VirtualUnlock(self.base_ptr.cast(), self.mem_size) {
                    warn!("Failed to unlock memory: {e}")
                }
            }
        }
    }

    fn get_set_size() -> anyhow::Result<(usize, usize)> {
        let (mut min_set_size, mut max_set_size) = (0, 0);
        unsafe {
            GetProcessWorkingSetSize(GetCurrentProcess(), &mut min_set_size, &mut max_set_size)
                .context("failed to get process working set")?;
        }
        Ok((min_set_size, max_set_size))
    }
}

#[cfg(unix)]
mod unix {
    use {
        crate::{prelude::*, MemLockGuard},
        libc::{getrlimit, mlock, munlock, rlimit, sysconf, RLIMIT_MEMLOCK, _SC_PAGESIZE},
        std::{
            borrow::BorrowMut,
            cmp,
            io::{Error, ErrorKind},
        },
    };

    pub(super) fn memory_resize_and_lock(
        mut memory: &mut [usize],
        allow_mem_resize: bool,
    ) -> anyhow::Result<(&mut [usize], MemLockGuard)> {
        let page_size = usize::try_from(unsafe { sysconf(_SC_PAGESIZE) }).unwrap();
        let usize_per_page = page_size / std::mem::size_of::<usize>();

        loop {
            let base_ptr = memory.as_mut_ptr();
            let mem_size = size_of_val(memory);
            if unsafe { mlock(base_ptr.cast(), mem_size) } == 0 {
                info!("Successfully locked {}MB", mem_size);
                return Ok((
                    memory,
                    MemLockGuard {
                        base_ptr: base_ptr.cast(),
                        mem_size,
                    },
                ));
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

            // Set new_len to memory locking system limit if this is the first resize, otherwise
            // decrement by a page.
            // Note that locking with system limit can fail because the memory might not be page
            // aligned
            let new_len = cmp::min(
                get_max_mem_lock()? / size_of::<usize>(),
                memory
                    .len()
                    .checked_sub(usize_per_page)
                    .context("Failed to lock any memory, memory size has been decremented to 0")?,
            );

            memory = &mut memory[0..new_len];
            warn!(
                "Decremented memory size to {}MB, retry memory locking",
                new_len * usize_per_page
            );
        }
    }

    fn get_max_mem_lock() -> anyhow::Result<usize> {
        unsafe {
            let mut rlim: rlimit = std::mem::zeroed();
            ensure!(
                getrlimit(RLIMIT_MEMLOCK, rlim.borrow_mut()) == 0,
                anyhow!(Error::last_os_error()).context("Failed to get RLIMIT_MEMLOCK")
            );
            Ok(rlim.rlim_cur.try_into().unwrap())
        }
    }

    impl Drop for MemLockGuard {
        fn drop(&mut self) {
            unsafe {
                if munlock(self.base_ptr.cast(), self.mem_size) != 0 {
                    warn!("Failed to unlock memory: {}", Error::last_os_error())
                }
            }
        }
    }
}
