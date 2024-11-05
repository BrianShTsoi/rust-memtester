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
    /// This is used by tests where memory is splitted in two and random values are written in pairs
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
    OwnAddress,
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

/// Write the address of each memory location to itself
/// then read back the value and verify that it matches the expected address
#[tracing::instrument(skip_all)]
pub unsafe fn test_own_address(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    // TODO: According to the linux memtester, this needs to be run several times,
    //       and with alternating complements of address
    let expected_iter = u64::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(2))
        .context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    for i in 0..count {
        timeout_checker.check()?;
        let ptr = base_ptr.add(i);
        write_volatile(ptr, ptr as usize);
    }

    for i in 0..count {
        timeout_checker.check()?;
        let ptr = base_ptr.add(i);
        let val = read_volatile(ptr);
        let address = ptr as usize;

        if val != address {
            info!("Test failed at {ptr:?}");
            return Ok(MemtestOutcome::Fail(MemtestFailure::UnexpectedValue {
                address,
                expected: address,
                actual: val,
            }));
        }
    }
    Ok(MemtestOutcome::Pass)
}

/// Split specified memory region into two halves and iterate through memory locations in pairs
/// write a random value for each pair of memory locations
/// read and compare the two halves of the memory region
#[tracing::instrument(skip_all)]
pub unsafe fn test_random_val(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter =
        u64::try_from(half_count * 2).context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    for i in 0..half_count {
        timeout_checker.check()?;
        let val = random();
        write_volatile(base_ptr.add(i), val);
        write_volatile(half_ptr.add(i), val);
    }
    compare_regions(base_ptr, half_ptr, half_count, timeout_checker)
}

/// Reset all bytes in specified memory region to 0xff
/// Split specified memory region into two halves and iterate through memory locations in pairs
/// For each pair, write the XOR result of a random value and the value read from the location
/// Read and compare the two halves of the memory region
#[tracing::instrument(skip_all)]
pub unsafe fn test_xor(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    let write_xor = |ptr1: *mut usize, ptr2: *mut usize, val| {
        write_volatile(ptr1, val ^ read_volatile(ptr1));
        write_volatile(ptr2, val ^ read_volatile(ptr2));
    };
    test_two_regions(base_ptr, count, timeout_checker, write_xor)
}

/// Reset all bytes in specified memory region to 0xff
/// Split specified memory region into two halves and iterate through memory locations in pairs
/// For each pair, write the result of subtracting a random value from the value read from the location
/// Read and compare the two halves of the memory region
#[tracing::instrument(skip_all)]
pub unsafe fn test_sub(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    let write_sub = |ptr1: *mut usize, ptr2: *mut usize, val| {
        write_volatile(ptr1, read_volatile(ptr1).wrapping_sub(val));
        write_volatile(ptr2, read_volatile(ptr2).wrapping_sub(val));
    };
    test_two_regions(base_ptr, count, timeout_checker, write_sub)
}

/// Reset all bytes in specified memory region to 0xff
/// Split specified memory region into two halves and iterate through memory locations in pairs
/// For each pair, write the result of multiplying a random value from the value read from the location
/// Read and compare the two halves of the memory region
#[tracing::instrument(skip_all)]
pub unsafe fn test_mul(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    let write_mul = |ptr1: *mut usize, ptr2: *mut usize, val| {
        write_volatile(ptr1, read_volatile(ptr1).wrapping_mul(val));
        write_volatile(ptr2, read_volatile(ptr2).wrapping_mul(val));
    };
    test_two_regions(base_ptr, count, timeout_checker, write_mul)
}

/// Reset all bytes in specified memory region to 0xff
/// Split specified memory region into two halves and iterate through memory locations in pairs
/// For each pair, write the result of dividing a random value from the value read from the location
/// Read and compare the two halves of the memory region
#[tracing::instrument(skip_all)]
pub unsafe fn test_div(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    let write_div = |ptr1: *mut usize, ptr2: *mut usize, val| {
        let val = if val == 0 { 1 } else { val };
        write_volatile(ptr1, read_volatile(ptr1) / val);
        write_volatile(ptr2, read_volatile(ptr2) / val);
    };
    test_two_regions(base_ptr, count, timeout_checker, write_div)
}

/// Reset all bytes in specified memory region to 0xff
/// Split specified memory region into two halves and iterate through memory locations in pairs
/// For each pair, write the OR result of a random value and the value read from the location
/// Read and compare the two halves of the memory region
#[tracing::instrument(skip_all)]
pub unsafe fn test_or(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    let write_or = |ptr1: *mut usize, ptr2: *mut usize, val| {
        write_volatile(ptr1, read_volatile(ptr1) | val);
        write_volatile(ptr2, read_volatile(ptr2) | val);
    };
    test_two_regions(base_ptr, count, timeout_checker, write_or)
}

/// Reset all bytes in specified memory region to 0xff
/// Split specified memory region into two halves and iterate through memory locations in pairs
/// For each pair, write the AND result of a random value and the value read from the location
/// Read and compare the two halves of the memory region
#[tracing::instrument(skip_all)]
pub unsafe fn test_and(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    let write_and = |ptr1: *mut usize, ptr2: *mut usize, val| {
        write_volatile(ptr1, read_volatile(ptr1) & val);
        write_volatile(ptr2, read_volatile(ptr2) & val);
    };
    test_two_regions(base_ptr, count, timeout_checker, write_and)
}

/// Base function for `test_xor`, `test_sub`, `test_mul`, `test_div`, `test_or` and `test_and`
/// Reset all bytes in specified memory region to 0xff
/// Split specified memory region into two halves and iterate through memory locations in pairs
/// Write to each pair using the given `write_val` function
/// Read and compare the two halves of the memory region
unsafe fn test_two_regions(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
    write_val: unsafe fn(*mut usize, *mut usize, usize),
) -> Result<MemtestOutcome, MemtestError> {
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter =
        u64::try_from(half_count * 2).context("Total number of iterations overflowed")?;
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
    base_ptr_1: *const usize,
    base_ptr_2: *const usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    for i in 0..count {
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

/// Split specified memory region into two halves and iterate through memory locations in pairs
/// For each pair, write the result of adding a random value to the index of iteration
/// Read and compare the two halves of the memory region
#[tracing::instrument(skip_all)]
pub unsafe fn test_seq_inc(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter =
        u64::try_from(half_count * 2).context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    let val: usize = random();
    for i in 0..half_count {
        timeout_checker.check()?;
        write_volatile(base_ptr.add(i), val.wrapping_add(i));
        write_volatile(half_ptr.add(i), val.wrapping_add(i));
    }
    compare_regions(base_ptr, half_ptr, half_count, timeout_checker)
}

/// Split specified memory region into two halves and iterate through memory locations in pairs
/// For each pair, write to all bytes with either 0xff or 0x0 in an alternating pattern
/// Read and compare the two halves of the memory region
/// This procedure is repeated 64 times
#[tracing::instrument(skip_all)]
pub unsafe fn test_solid_bits(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    const NUM_RUNS: u64 = 64;
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter = u64::try_from(half_count * 2)
        .ok()
        .and_then(|count| count.checked_mul(NUM_RUNS))
        .context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    for i in 0..64 {
        let mut val = if i % 2 == 0 { 0 } else { !0 };
        for j in 0..half_count {
            timeout_checker.check()?;
            val = !val;
            write_volatile(base_ptr.add(j), val);
            write_volatile(half_ptr.add(j), val);
        }
        compare_regions(base_ptr, half_ptr, half_count, timeout_checker)?;
    }
    Ok(MemtestOutcome::Pass)
}

/// Split specified memory region into two halves and iterate through memory locations in pairs
/// For each pair, write to all bytes with either 0x55 or 0xaa in an alternating pattern
/// Read and compare the two halves of the memory region
/// This procedure is repeated 64 times
#[tracing::instrument(skip_all)]
pub unsafe fn test_checkerboard(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    const NUM_RUNS: u64 = 64;
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter = u64::try_from(half_count * 2)
        .ok()
        .and_then(|count| count.checked_mul(NUM_RUNS))
        .context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    let mut checker_board: usize = 0;
    write_bytes(&mut checker_board, 0x55, 1);

    for i in 0..NUM_RUNS {
        let mut val = if i % 2 == 0 {
            checker_board
        } else {
            !checker_board
        };
        for j in 0..half_count {
            timeout_checker.check()?;
            val = !val;
            write_volatile(base_ptr.add(j), val);
            write_volatile(half_ptr.add(j), val);
        }
        compare_regions(base_ptr, half_ptr, half_count, timeout_checker)?;
    }
    Ok(MemtestOutcome::Pass)
}

/// Split specified memory region into two halves and iterate through memory locations in pairs
/// For each pair, write to all bytes with the value i in an alternating pattern
/// Read and compare the two halves of the memory region
/// This procedure is repeated 256 times, with i corresponding to the iteration number 0-255
#[tracing::instrument(skip_all)]
pub unsafe fn test_block_seq(
    base_ptr: *mut usize,
    count: usize,
    timeout_checker: &mut TimeoutChecker,
) -> Result<MemtestOutcome, MemtestError> {
    const NUM_RUNS: u64 = 256;
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    let expected_iter = u64::try_from(half_count * 2)
        .ok()
        .and_then(|count| count.checked_mul(NUM_RUNS))
        .context("Total number of iterations overflowed")?;
    timeout_checker.init(expected_iter);

    for i in 0..=(u8::try_from(NUM_RUNS - 1).unwrap()) {
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

// TODO: Consider a more readable format dipslaying MemtestOutcome
impl fmt::Display for MemtestOutcome {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl fmt::Display for MemtestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
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
