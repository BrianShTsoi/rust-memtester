use std::{
    ptr::{read_volatile, write_bytes, write_volatile},
    time::{Duration, Instant},
};

use rand::random;

#[derive(Debug)]
pub struct MemtestReport {
    pub test_type: MemtestType,
    pub outcome: Result<MemtestOutcome, MemtestError>,
}

#[derive(Debug)]
pub enum MemtestOutcome {
    Pass,
    Fail(usize, Option<usize>),
}

#[derive(Debug)]
pub enum MemtestError {
    Timeout,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
pub enum MemtestType {
    TestOwnAddress,
    TestRandomVal,
    TestXor,
    TestSub,
    TestMul,
    TestDiv,
    TestOr,
    TestAnd,
    TestSeqInc,
    TestSolidBits,
    TestCheckerboard,
    TestBlockSeq,
}

/// An interanl structure to ensure the test timeouts in a given time frame
#[derive(Debug)]
struct TimeoutChecker {
    start_time: Instant,
    timeout_ms: usize,
    total_iter: usize,
    completed_iter: usize,
    checkpoint: usize,
    num_checks_completed: u128,
    checking_interval_ns: f64,
}

// TODO: Logging?

pub unsafe fn test_own_address(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    // TODO:
    // According to the linux memtester, this needs to be run several times,
    // and with alternating complements of address
    let mut timeout_checker = TimeoutChecker::new(Instant::now(), timeout_ms, count * 2);
    for i in 0..count {
        timeout_checker.check()?;
        let ptr = base_ptr.add(i);
        write_volatile(ptr, ptr as usize);
    }

    for i in 0..count {
        timeout_checker.check()?;
        let ptr = base_ptr.add(i);
        if read_volatile(ptr) != ptr as usize {
            return Ok(MemtestOutcome::Fail(ptr as usize, None));
        }
    }
    Ok(MemtestOutcome::Pass)
}

pub unsafe fn test_random_val(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let mut timeout_checker = TimeoutChecker::new(Instant::now(), timeout_ms, half_count * 2);
    for i in 0..half_count {
        timeout_checker.check()?;
        let value = random();
        write_volatile(base_ptr.add(i), value);
        write_volatile(half_ptr.add(i), value);
    }
    compare_regions(base_ptr, half_ptr, half_count, &mut timeout_checker)
}

pub unsafe fn test_xor(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(base_ptr, count, timeout_ms, write_xor)
}

pub unsafe fn test_sub(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(base_ptr, count, timeout_ms, write_sub)
}

pub unsafe fn test_mul(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(base_ptr, count, timeout_ms, write_mul)
}

pub unsafe fn test_div(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(base_ptr, count, timeout_ms, write_div)
}

pub unsafe fn test_or(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(base_ptr, count, timeout_ms, write_or)
}

pub unsafe fn test_and(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(base_ptr, count, timeout_ms, write_and)
}

unsafe fn test_two_regions<F>(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
    write_val: F,
) -> Result<MemtestOutcome, MemtestError>
where
    F: Fn(*mut usize, *mut usize, usize),
{
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let mut timeout_checker = TimeoutChecker::new(Instant::now(), timeout_ms, half_count * 2);
    mem_reset(base_ptr, count);

    for i in 0..half_count {
        timeout_checker.check()?;
        write_val(base_ptr.add(i), half_ptr.add(i), random());
    }

    compare_regions(base_ptr, half_ptr, half_count, &mut timeout_checker)
}

unsafe fn mem_reset(base_ptr: *mut usize, count: usize) {
    write_bytes(base_ptr, 0xff, count);
}

unsafe fn compare_regions(
    ptr1: *const usize,
    ptr2: *const usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    for i in 0..count {
        timeout_checker.check()?;
        if read_volatile(ptr1.add(i)) != read_volatile(ptr2.add(i)) {
            return Ok(MemtestOutcome::Fail(
                ptr1.add(i) as usize,
                Some(ptr2.add(i) as usize),
            ));
        }
    }
    Ok(MemtestOutcome::Pass)
}

fn write_xor(ptr1: *mut usize, ptr2: *mut usize, val: usize) {
    unsafe {
        write_volatile(ptr1, val ^ read_volatile(ptr1));
        write_volatile(ptr2, val ^ read_volatile(ptr2));
    }
}

fn write_sub(ptr1: *mut usize, ptr2: *mut usize, val: usize) {
    unsafe {
        write_volatile(ptr1, read_volatile(ptr1).wrapping_sub(val));
        write_volatile(ptr2, read_volatile(ptr2).wrapping_sub(val));
    }
}

fn write_mul(ptr1: *mut usize, ptr2: *mut usize, val: usize) {
    unsafe {
        write_volatile(ptr1, read_volatile(ptr1).wrapping_mul(val));
        write_volatile(ptr2, read_volatile(ptr2).wrapping_mul(val));
    }
}
fn write_div(ptr1: *mut usize, ptr2: *mut usize, val: usize) {
    let val = if val == 0 { 1 } else { val };
    unsafe {
        write_volatile(ptr1, read_volatile(ptr1) / val);
        write_volatile(ptr2, read_volatile(ptr2) / val);
    }
}

fn write_or(ptr1: *mut usize, ptr2: *mut usize, val: usize) {
    unsafe {
        write_volatile(ptr1, read_volatile(ptr1) | val);
        write_volatile(ptr2, read_volatile(ptr2) | val);
    }
}

fn write_and(ptr1: *mut usize, ptr2: *mut usize, val: usize) {
    unsafe {
        write_volatile(ptr1, read_volatile(ptr1) & val);
        write_volatile(ptr2, read_volatile(ptr2) & val);
    }
}

pub unsafe fn test_seq_inc(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let mut timeout_checker = TimeoutChecker::new(Instant::now(), timeout_ms, half_count * 2);

    let value: usize = random();
    for i in 0..half_count {
        timeout_checker.check()?;
        write_volatile(base_ptr.add(i), value.wrapping_add(i));
        write_volatile(half_ptr.add(i), value.wrapping_add(i));
    }
    compare_regions(base_ptr, half_ptr, half_count, &mut timeout_checker)
}

pub unsafe fn test_solid_bits(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let mut timeout_checker = TimeoutChecker::new(Instant::now(), timeout_ms, half_count * 256);

    for i in 0..64 {
        let val = if i % 2 == 0 { !0 } else { 0 };
        for j in 0..half_count {
            timeout_checker.check()?;
            let val = if j % 2 == 0 { val } else { !val };
            write_volatile(base_ptr.add(j), val);
            write_volatile(half_ptr.add(j), val);
        }
        compare_regions(base_ptr, half_ptr, half_count, &mut timeout_checker)?;
    }
    Ok(MemtestOutcome::Pass)
}

pub unsafe fn test_checkerboard(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    const CHECKER_BOARD: usize = 0x5555555555555555;
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let mut timeout_checker = TimeoutChecker::new(Instant::now(), timeout_ms, half_count * 2 * 64);

    for i in 0..64 {
        let val = if i % 2 == 0 {
            CHECKER_BOARD
        } else {
            !CHECKER_BOARD
        };
        for j in 0..half_count {
            timeout_checker.check()?;
            let val = if j % 2 == 0 { val } else { !val };
            write_volatile(base_ptr.add(j), val);
            write_volatile(half_ptr.add(j), val);
        }
        compare_regions(base_ptr, half_ptr, half_count, &mut timeout_checker)?;
    }
    Ok(MemtestOutcome::Pass)
}

pub unsafe fn test_block_seq(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let mut timeout_checker = TimeoutChecker::new(Instant::now(), timeout_ms, half_count * 2 * 256);

    for i in 0..=255 {
        let mut val: usize = 0;
        write_bytes(&mut val, i, 1);
        for j in 0..half_count {
            timeout_checker.check()?;
            write_volatile(base_ptr.add(j), val);
            write_volatile(half_ptr.add(j), val);
        }
        compare_regions(base_ptr, half_ptr, half_count, &mut timeout_checker)?;
    }
    Ok(MemtestOutcome::Pass)
}

impl TimeoutChecker {
    fn new(start_time: Instant, timeout_ms: usize, total_iter: usize) -> TimeoutChecker {
        TimeoutChecker {
            start_time,
            timeout_ms,
            total_iter,
            completed_iter: 0,
            checkpoint: 1,
            num_checks_completed: 0,
            // TODO: Choice of starting interval is arbitrary for now.
            checking_interval_ns: 1000.0,
        }
    }

    /// This function should be called in every iteration of a memtest
    ///
    /// To minimize overhead, the time is only checked at specific checkpoints.
    /// The algorithm makes a prediction of how many iterations will be done during `checking_interval_ns`
    /// and sets the checkpoint based on the prediction.
    ///
    /// The algorithm also determines whether it is likely that the test will be completed
    /// based on `work_progress` and `time_progress`.
    /// If it is likely that the test will be completed, `checking_interval_ns` is scaled up to be more
    /// lenient and reduce overhead.
    fn check(&mut self) -> Result<(), MemtestError> {
        if self.completed_iter < self.checkpoint {
            self.completed_iter += 1;
            return Ok(());
        }

        let elapsed = self.start_time.elapsed();
        if elapsed > Duration::from_millis(self.timeout_ms as u64) {
            return Err(MemtestError::Timeout);
        }

        let work_progress = self.completed_iter as f64 / self.total_iter as f64;
        let time_progress = elapsed.as_millis() as f64 / self.timeout_ms as f64;
        // TODO: Consider having a max for `checking_interval_ns` to have a reasonable timeout guarantee
        if work_progress > time_progress {
            self.checking_interval_ns *= 2.0;
        }

        let avg_iter_time_ns = elapsed.as_nanos() as f64 / self.completed_iter as f64;
        let iter_per_interval = self.checking_interval_ns / avg_iter_time_ns;
        self.checkpoint = self.completed_iter + iter_per_interval as usize;

        self.num_checks_completed += 1;
        self.completed_iter += 1;
        Ok(())
    }
}

impl MemtestReport {
    pub(super) fn new(
        test_type: MemtestType,
        outcome: Result<MemtestOutcome, MemtestError>,
    ) -> MemtestReport {
        MemtestReport { test_type, outcome }
    }
}
