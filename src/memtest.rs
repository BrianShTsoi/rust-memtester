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

#[derive(Debug)]
pub enum MemtestOutcome {
    Pass,
    Fail(MemtestFailure),
}

#[derive(Debug)]
pub enum MemtestFailure {
    /// Failure due to the actual value read being different from the expected value
    UnexpectedValue {
        address: usize,
        expected: usize,
        actual: usize,
    },
    /// Failure due to the two memory locations being compared returning two different values
    /// This is used by tests where memory is split in two and values are written in pairs
    MismatchedValues {
        address1: usize,
        value1: usize,
        address2: usize,
        value2: usize,
    },
}

#[derive(Debug)]
pub enum MemtestError {
    Timeout,
    Other(anyhow::Error),
}

#[derive(Clone, Copy, Debug)]
pub enum MemtestKind {
    OwnAddressBasic,
    OwnAddressRepeat,
    RandomVal,
    Xor,
    Sub,
    Mul,
    Div,
    Or,
    And,
    SeqInc,
    SolidBits,
    Checkerboard,
    BlockSeq,
}

/// Write the address of each memory location to itself, then read back the value and check that it
/// matches the expected address.
#[tracing::instrument(skip_all)]
pub fn test_own_address_basic(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    let base_ptr = memory.as_mut_ptr();
    let len = memory.len();
    let expected_iter = u64::try_from(len)
        .ok()
        .and_then(|count| count.checked_mul(2))
        .context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    for i in 0..len {
        timeout_checker.check()?;
        unsafe {
            let ptr = base_ptr.add(i);
            write_volatile(ptr, ptr as usize);
        }
    }

    for i in 0..len {
        timeout_checker.check()?;
        let ptr = unsafe { base_ptr.add(i) };
        let address = ptr as usize;
        let actual = unsafe { read_volatile(ptr) };

        if actual != address {
            info!("Test failed at {ptr:?}");
            return Ok(MemtestOutcome::Fail(MemtestFailure::UnexpectedValue {
                address,
                expected: address,
                actual,
            }));
        }
    }
    Ok(MemtestOutcome::Pass)
}

/// Write the address of each memory location (or its complement) to itself, then read back the
/// value and check that it matches the expected address.
/// This procedure is repeated 16 times.
pub fn test_own_address_repeat(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    const NUM_RUNS: u64 = 16;
    let base_ptr = memory.as_mut_ptr();
    let len = memory.len();
    let expected_iter = u64::try_from(len)
        .ok()
        .and_then(|count| count.checked_mul(2 * NUM_RUNS))
        .context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    let write_val = |address: usize, i, j| {
        if (i + j) % 2 == 0 {
            address
        } else {
            !(address)
        }
    };

    for i in 0..usize::try_from(NUM_RUNS).unwrap() {
        for j in 0..len {
            timeout_checker.check()?;
            let ptr = unsafe { base_ptr.add(j) };
            let address = ptr as usize;
            let val = write_val(address, i, j);
            unsafe {
                write_volatile(ptr, val);
            }
        }

        for j in 0..len {
            timeout_checker.check()?;
            let ptr = unsafe { base_ptr.add(j) };
            let address = ptr as usize;
            let expected = write_val(address, i, j);
            let actual = unsafe { read_volatile(ptr) };

            if actual != expected {
                info!("Test failed at {ptr:?}");
                return Ok(MemtestOutcome::Fail(MemtestFailure::UnexpectedValue {
                    address,
                    expected,
                    actual,
                }));
            }
        }
    }

    Ok(MemtestOutcome::Pass)
}

/// Split given memory into two halves and iterate through memory locations in pairs. For each
/// pair, write a random value. After all locations are written, read and compare the two halves.
#[tracing::instrument(skip_all)]
pub fn test_random_val(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    let base_ptr = memory.as_mut_ptr();
    let half_len = memory.len() / 2;
    let half_ptr = unsafe { base_ptr.add(half_len) };
    let expected_iter =
        u64::try_from(half_len * 2).context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    for i in 0..half_len {
        timeout_checker.check()?;
        let val = random();
        unsafe {
            write_volatile(base_ptr.add(i), val);
            write_volatile(half_ptr.add(i), val);
        }
    }
    unsafe { compare_regions(base_ptr, half_ptr, half_len, timeout_checker) }
}

/// Reset all bits in given memory to 1s. Split given memory into two halves and iterate through
/// memory locations in pairs. For each pair, write the XOR result of a random value and the value
/// read from the location. After all locations are written, read and compare the two halves.
#[tracing::instrument(skip_all)]
pub fn test_xor(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(memory, timeout_checker, |ptr1, ptr2, val| unsafe {
        write_volatile(ptr1, val ^ read_volatile(ptr1));
        write_volatile(ptr2, val ^ read_volatile(ptr2));
    })
}

/// Reset all bits in given memory to 1s. Split given memory into two halves and iterate through
/// memory locations in pairs. For each pair, write the result of subtracting a random value from
/// the value read from the location. After all locations are written, read and compare the two
/// halves.
#[tracing::instrument(skip_all)]
pub fn test_sub(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(memory, timeout_checker, |ptr1, ptr2, val| unsafe {
        write_volatile(ptr1, read_volatile(ptr1).wrapping_sub(val));
        write_volatile(ptr2, read_volatile(ptr2).wrapping_sub(val));
    })
}

/// Reset all bits in given memory to 1s. Split given memory into two halves and iterate through
/// memory locations in pairs. For each pair, write the result of multiplying a random value with
/// the value read from the location. After all locations are written, read and compare the two
/// halves.
#[tracing::instrument(skip_all)]
pub fn test_mul(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(memory, timeout_checker, |ptr1, ptr2, val| unsafe {
        write_volatile(ptr1, read_volatile(ptr1).wrapping_mul(val));
        write_volatile(ptr2, read_volatile(ptr2).wrapping_mul(val));
    })
}

/// Reset all bits in given memory to 1s. Split given memory into two halves and iterate through
/// memory locations in pairs. For each pair, write the result of dividing the value read from the
/// location with a random value. After all locations are written, read and compare the two halves.
#[tracing::instrument(skip_all)]
pub fn test_div(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(memory, timeout_checker, |ptr1, ptr2, val| unsafe {
        let val = if val == 0 { 1 } else { val };
        write_volatile(ptr1, read_volatile(ptr1) / val);
        write_volatile(ptr2, read_volatile(ptr2) / val);
    })
}

/// Reset all bits in given memory to 1s. Split given memory into two halves and iterate through
/// memory locations in pairs. For each pair, write the OR result of a random value and the value
/// read from the location. After all locations are written, read and compare the two halves.
#[tracing::instrument(skip_all)]
pub fn test_or(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(memory, timeout_checker, |ptr1, ptr2, val| unsafe {
        write_volatile(ptr1, read_volatile(ptr1) | val);
        write_volatile(ptr2, read_volatile(ptr2) | val);
    })
}

/// Reset all bits in given memory to 1s. Split given memory into two halves and iterate through
/// memory locations in pairs. For each pair, write the AND result of a random value and the value
/// read from the location. After all locations are written, read and compare the two halves.
#[tracing::instrument(skip_all)]
pub fn test_and(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    test_two_regions(memory, timeout_checker, |ptr1, ptr2, val| unsafe {
        write_volatile(ptr1, read_volatile(ptr1) & val);
        write_volatile(ptr2, read_volatile(ptr2) & val);
    })
}

/// Base function for `test_xor`, `test_sub`, `test_mul`, `test_div`, `test_or` and `test_and`
///
/// Reset all bits in given memory to 1s. Split given memory into two halves and iterate through
/// memory locations in pairs. Write to each pair using the given `write_val` function. After all
/// locations are written, read and compare the two halves.
fn test_two_regions(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
    write_val: unsafe fn(*mut usize, *mut usize, usize),
) -> Result<MemtestOutcome, MemtestError> {
    ensure_two_regions_mem_len(memory)?;
    mem_reset(memory);
    let base_ptr = memory.as_mut_ptr();
    let half_len = memory.len() / 2;
    let half_ptr = unsafe { base_ptr.add(half_len) };
    let expected_iter =
        u64::try_from(half_len * 2).context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    for i in 0..half_len {
        timeout_checker.check()?;
        unsafe { write_val(base_ptr.add(i), half_ptr.add(i), random()) };
    }

    unsafe { compare_regions(base_ptr, half_ptr, half_len, timeout_checker) }
}

fn ensure_two_regions_mem_len(memory: &mut [usize]) -> Result<(), MemtestError> {
    (memory.len() >= 2)
        .then_some(())
        .ok_or(MemtestError::Other(anyhow!(
            "Insufficient memory length for two-regions memtest"
        )))
}

fn mem_reset(memory: &mut [usize]) {
    let mut reset_val: usize = 0;
    unsafe { write_bytes(&mut reset_val, 0xff, 1) };
    for i in 0..memory.len() {
        unsafe {
            write_volatile(memory.as_mut_ptr().add(i), reset_val);
        }
    }
}

unsafe fn compare_regions(
    base_ptr_1: *const usize,
    base_ptr_2: *const usize,
    len: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    for i in 0..len {
        timeout_checker.check()?;

        let ptr1 = base_ptr_1.add(i);
        let ptr2 = base_ptr_2.add(i);
        let val1 = read_volatile(ptr1);
        let val2 = read_volatile(ptr2);

        if val1 != val2 {
            info!("Test failed at {ptr1:?} compared to {ptr2:?}");
            return Ok(MemtestOutcome::Fail(MemtestFailure::MismatchedValues {
                address1: ptr1 as usize,
                value1: val1,
                address2: ptr2 as usize,
                value2: val2,
            }));
        }
    }
    Ok(MemtestOutcome::Pass)
}

/// Split given memory into two halves and iterate through memory locations in pairs. For each
/// pair, write the result of adding a random value to the index of iteration. After all locations
/// are written, read and compare the two halves.
#[tracing::instrument(skip_all)]
pub fn test_seq_inc(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    ensure_two_regions_mem_len(memory)?;
    let base_ptr = memory.as_mut_ptr();
    let half_len = memory.len() / 2;
    let half_ptr = unsafe { base_ptr.add(half_len) };
    let expected_iter =
        u64::try_from(half_len * 2).context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    let val: usize = random();
    for i in 0..half_len {
        timeout_checker.check()?;
        unsafe {
            write_volatile(base_ptr.add(i), val.wrapping_add(i));
            write_volatile(half_ptr.add(i), val.wrapping_add(i));
        }
    }
    unsafe { compare_regions(base_ptr, half_ptr, half_len, timeout_checker) }
}

/// Split given memory into two halves and iterate through memory locations in pairs. For each
/// pair, write to all bits as either 1s or 0s, alternating after each memory location pair.
/// After all locations are written, read and compare the two halves.
/// This procedure is repeated 64 times
#[tracing::instrument(skip_all)]
pub fn test_solid_bits(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    ensure_two_regions_mem_len(memory)?;
    const NUM_RUNS: u64 = 64;
    let base_ptr = memory.as_mut_ptr();
    let half_len = memory.len() / 2;
    let half_ptr = unsafe { base_ptr.add(half_len) };
    let expected_iter = u64::try_from(half_len * 2)
        .ok()
        .and_then(|count| count.checked_mul(NUM_RUNS))
        .context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    for i in 0..NUM_RUNS {
        let mut val = if i % 2 == 0 { 0 } else { !0 };
        for j in 0..half_len {
            timeout_checker.check()?;
            val = !val;
            unsafe {
                write_volatile(base_ptr.add(j), val);
                write_volatile(half_ptr.add(j), val);
            }
        }
        unsafe {
            compare_regions(base_ptr, half_ptr, half_len, timeout_checker)?;
        }
    }
    Ok(MemtestOutcome::Pass)
}

/// Split given memory into two halves and iterate through memory locations in pairs. For each pair,
/// write to a pattern of alternating 1s and 0s (in bytes it is either 0x55 or 0xaa, and alternating
/// after each memory location pair). After all locations are written, read and compare the two
/// halves.
/// This procedure is repeated 64 times
#[tracing::instrument(skip_all)]
pub fn test_checkerboard(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    ensure_two_regions_mem_len(memory)?;
    const NUM_RUNS: u64 = 64;
    let base_ptr = memory.as_mut_ptr();
    let half_len = memory.len() / 2;
    let half_ptr = unsafe { base_ptr.add(half_len) };
    let expected_iter = u64::try_from(half_len * 2)
        .ok()
        .and_then(|count| count.checked_mul(NUM_RUNS))
        .context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    let mut checker_board: usize = 0;
    unsafe { write_bytes(&mut checker_board, 0x55, 1) };

    for i in 0..NUM_RUNS {
        let mut val = if i % 2 == 0 {
            checker_board
        } else {
            !checker_board
        };
        for j in 0..half_len {
            timeout_checker.check()?;
            val = !val;
            unsafe {
                write_volatile(base_ptr.add(j), val);
                write_volatile(half_ptr.add(j), val);
            }
        }
        unsafe {
            compare_regions(base_ptr, half_ptr, half_len, timeout_checker)?;
        }
    }
    Ok(MemtestOutcome::Pass)
}

/// Split given memory into two halves and iterate through memory locations in pairs. For each pair,
/// write to all bytes with the value i. After all locations are written, read and compare the two
/// halves.
/// This procedure is repeated 256 times, with i corresponding to the iteration number 0-255.
#[tracing::instrument(skip_all)]
pub fn test_block_seq(
    memory: &mut [usize],
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    ensure_two_regions_mem_len(memory)?;
    const NUM_RUNS: u64 = 256;
    let base_ptr = memory.as_mut_ptr();
    let half_len = memory.len() / 2;
    let half_ptr = unsafe { base_ptr.add(half_len) };
    let expected_iter = u64::try_from(half_len * 2)
        .ok()
        .and_then(|count| count.checked_mul(NUM_RUNS))
        .context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    for i in 0..=(u8::try_from(NUM_RUNS - 1).unwrap()) {
        let mut val: usize = 0;
        unsafe {
            write_bytes(&mut val, i, 1);
        }
        for j in 0..half_len {
            timeout_checker.check()?;
            unsafe {
                write_volatile(base_ptr.add(j), val);
                write_volatile(half_ptr.add(j), val);
            }
        }
        unsafe {
            compare_regions(base_ptr, half_ptr, half_len, timeout_checker)?;
        }
    }
    Ok(MemtestOutcome::Pass)
}

impl fmt::Display for MemtestOutcome {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Outcome: {:?}", self)
    }
}

impl fmt::Display for MemtestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Error: {:?}", self)
    }
}

impl Error for MemtestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            MemtestError::Timeout => None,
            MemtestError::Other(err) => Some(err.as_ref()),
        }
    }
}

impl From<anyhow::Error> for MemtestError {
    fn from(err: anyhow::Error) -> MemtestError {
        MemtestError::Other(err)
    }
}
