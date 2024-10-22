use {
    crate::{prelude::*, TimeoutChecker},
    rand::random,
    std::{
        error::Error,
        fmt,
        ptr::{read_volatile, write_bytes, write_volatile},
    },
};

// TODO: Intend to convert this module to a standalone `no_std` crate
// TODO: TimeoutChecker will be a trait instead
// TODO: Can't use `anyhow` in `no_std`

// TODO: Show expected value of address if test failed?
// But this maybe be problematic for tests that only compare two regions
#[derive(Debug)]
pub enum MemtestOutcome {
    Pass,
    Fail(usize, Option<usize>),
}

#[derive(Debug)]
pub enum MemtestError {
    Timeout,
    Other(anyhow::Error),
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

pub unsafe fn test_own_address(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestOwnAddress");
    // TODO:
    // According to the linux memtester, this needs to be run several times,
    // and with alternating complements of address
    let expected_iter = u64::try_from(count.checked_mul(2).context("expected_iter overflowed")?)
        .context("Failed to convert expected_iter to u64")?;
    timeout_checker.init(expected_iter);

    for i in 0..count {
        timeout_checker.check()?;
        let ptr = base_ptr.add(i);
        write_volatile(ptr, ptr as usize);
    }

    for i in 0..count {
        timeout_checker.check()?;
        let ptr = base_ptr.add(i);
        if read_volatile(ptr) != ptr as usize {
            info!("Test failed at {ptr:?}");
            return Ok(MemtestOutcome::Fail(ptr as usize, None));
        }
    }
    Ok(MemtestOutcome::Pass)
}

pub unsafe fn test_random_val(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestRandomVal");
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter =
        u64::try_from(half_count * 2).context("Failed to convert expected_iter to u64")?;
    timeout_checker.init(expected_iter);

    for i in 0..half_count {
        timeout_checker.check()?;
        let value = random();
        write_volatile(base_ptr.add(i), value);
        write_volatile(half_ptr.add(i), value);
    }
    compare_regions(base_ptr, half_ptr, half_count, timeout_checker)
}

pub unsafe fn test_xor(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestXor");
    test_two_regions(base_ptr, count, timeout_checker, write_xor)
}

pub unsafe fn test_sub(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestSub");
    test_two_regions(base_ptr, count, timeout_checker, write_sub)
}

pub unsafe fn test_mul(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestMul");
    test_two_regions(base_ptr, count, timeout_checker, write_mul)
}

pub unsafe fn test_div(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestDiv");
    test_two_regions(base_ptr, count, timeout_checker, write_div)
}

pub unsafe fn test_or(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestOr");
    test_two_regions(base_ptr, count, timeout_checker, write_or)
}

pub unsafe fn test_and(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestAnd");
    test_two_regions(base_ptr, count, timeout_checker, write_and)
}

unsafe fn test_two_regions<F>(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
    write_val: F,
) -> Result<MemtestOutcome, MemtestError>
where
    F: Fn(*mut usize, *mut usize, usize),
{
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter =
        u64::try_from(half_count * 2).context("Failed to convert expected_iter to u64")?;
    timeout_checker.init(expected_iter);
    mem_reset(base_ptr, count);

    for i in 0..half_count {
        timeout_checker.check()?;
        write_val(base_ptr.add(i), half_ptr.add(i), random());
    }

    compare_regions(base_ptr, half_ptr, half_count, timeout_checker)
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
            info!("Test failed at {ptr1:?} compared to {ptr2:?}");
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
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestSeqInc");
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter =
        u64::try_from(half_count * 2).context("Failed to convert expected_iter to u64")?;
    timeout_checker.init(expected_iter);

    let value: usize = random();
    for i in 0..half_count {
        timeout_checker.check()?;
        write_volatile(base_ptr.add(i), value.wrapping_add(i));
        write_volatile(half_ptr.add(i), value.wrapping_add(i));
    }
    compare_regions(base_ptr, half_ptr, half_count, timeout_checker)
}

pub unsafe fn test_solid_bits(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestSolidBits");
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter = u64::try_from(
        (half_count * 2)
            .checked_mul(64)
            .context("expected_iter overflowed")?,
    )
    .context("Failed to convert expected_iter to u64")?;
    timeout_checker.init(expected_iter);

    for i in 0..64 {
        let val = if i % 2 == 0 { !0 } else { 0 };
        for j in 0..half_count {
            timeout_checker.check()?;
            let val = if j % 2 == 0 { val } else { !val };
            write_volatile(base_ptr.add(j), val);
            write_volatile(half_ptr.add(j), val);
        }
        compare_regions(base_ptr, half_ptr, half_count, timeout_checker)?;
    }
    Ok(MemtestOutcome::Pass)
}

pub unsafe fn test_checkerboard(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestCheckerboard");
    const CHECKER_BOARD: usize = 0x5555555555555555;
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter = u64::try_from(
        (half_count * 2)
            .checked_mul(64)
            .context("expected_iter overflowed")?,
    )
    .context("Failed to convert expected_iter to u64")?;
    timeout_checker.init(expected_iter);

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
        compare_regions(base_ptr, half_ptr, half_count, timeout_checker)?;
    }
    Ok(MemtestOutcome::Pass)
}

pub unsafe fn test_block_seq(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    debug!("Running TestBlockSeq");
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter = u64::try_from(
        (half_count * 2)
            .checked_mul(256)
            .context("expected_iter overflowed")?,
    )
    .context("Failed to convert expected_iter to u64")?;
    timeout_checker.init(expected_iter);

    for i in 0..=255 {
        let mut val: usize = 0;
        write_bytes(&mut val, i, 1);
        for j in 0..half_count {
            timeout_checker.check()?;
            write_volatile(base_ptr.add(j), val);
            write_volatile(half_ptr.add(j), val);
        }
        compare_regions(base_ptr, half_ptr, half_count, timeout_checker)?;
    }
    Ok(MemtestOutcome::Pass)
}

// TODO: More proper Display and Error impl for MemtestError?

impl fmt::Display for MemtestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for MemtestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            MemtestError::Timeout => None,
            MemtestError::Other(err) => err.source(),
        }
    }
}

impl From<anyhow::Error> for MemtestError {
    fn from(err: anyhow::Error) -> MemtestError {
        MemtestError::Other(err)
    }
}
