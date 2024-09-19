use std::{
    mem::size_of,
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
    Fail(usize),
}

#[derive(Debug)]
pub enum MemtestError {
    Timeout,
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

// TODO: Logging?
// TODO: if a more precise timeout is required, consider async
// TODO: if time taken to run a test is too long,
//       consider testing memory regions in smaller chunks

// TODO:
// According to memtester, this needs to be run several times,
// and with alternating complements of aaddress
pub unsafe fn test_own_address(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    let start_time = Instant::now();
    for i in 0..count {
        check_timeout(start_time, timeout_ms)?;
        let ptr = base_ptr.add(i);
        write_volatile(ptr, ptr as usize);
    }

    for i in 0..count {
        check_timeout(start_time, timeout_ms)?;
        let ptr = base_ptr.add(i);
        if read_volatile(ptr) != ptr as usize {
            return Ok(MemtestOutcome::Fail(ptr as usize));
        }
    }
    Ok(MemtestOutcome::Pass)
}

pub unsafe fn test_random_val(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    let start_time = Instant::now();
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);
    for i in 0..half_count {
        check_timeout(start_time, timeout_ms)?;
        let value = random();
        write_volatile(base_ptr.add(i), value);
        write_volatile(half_ptr.add(i), value);
    }
    compare_regions(base_ptr, half_ptr, half_count, start_time, timeout_ms)
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
    let start_time = Instant::now();
    write_bytes(base_ptr, 0xff, count);

    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);

    for i in 0..half_count {
        check_timeout(start_time, timeout_ms)?;
        write_val(base_ptr.add(i), half_ptr.add(i), random());
    }

    compare_regions(base_ptr, half_ptr, half_count, start_time, timeout_ms)
}

unsafe fn compare_regions(
    ptr1: *const usize,
    ptr2: *const usize,
    count: usize,
    start_time: Instant,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    for i in 0..count {
        check_timeout(start_time, timeout_ms)?;
        if read_volatile(ptr1.add(i)) != read_volatile(ptr2.add(i)) {
            return Ok(MemtestOutcome::Fail(ptr1 as usize));
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
    let start_time = Instant::now();
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);

    let value: usize = random();
    for i in 0..half_count {
        check_timeout(start_time, timeout_ms)?;
        write_volatile(base_ptr.add(i), value.wrapping_add(i));
        write_volatile(half_ptr.add(i), value.wrapping_add(i));
    }
    compare_regions(base_ptr, half_ptr, half_count, start_time, timeout_ms)
}

pub unsafe fn test_solid_bits(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    let start_time = Instant::now();
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);

    for i in 0..64 {
        let val = if i % 2 == 0 { !0 } else { 0 };
        for j in 0..half_count {
            check_timeout(start_time, timeout_ms)?;
            let val = if j % 2 == 0 { val } else { !val };
            write_volatile(base_ptr.add(j), val);
            write_volatile(half_ptr.add(j), val);
        }
        compare_regions(base_ptr, half_ptr, half_count, start_time, timeout_ms)?;
    }
    Ok(MemtestOutcome::Pass)
}

pub unsafe fn test_checkerboard(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    const CHECKER_BOARD: usize = 0x5555555555555555;
    let start_time = Instant::now();
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);

    for i in 0..64 {
        let val = if i % 2 == 0 {
            CHECKER_BOARD
        } else {
            !CHECKER_BOARD
        };
        for j in 0..half_count {
            check_timeout(start_time, timeout_ms)?;
            let val = if j % 2 == 0 { val } else { !val };
            write_volatile(base_ptr.add(j), val);
            write_volatile(half_ptr.add(j), val);
        }
        compare_regions(base_ptr, half_ptr, half_count, start_time, timeout_ms)?;
    }
    Ok(MemtestOutcome::Pass)
}

pub unsafe fn test_block_seq(
    base_ptr: *mut usize,
    count: usize,
    timeout_ms: usize,
) -> Result<MemtestOutcome, MemtestError> {
    let start_time = Instant::now();
    let half_count = count / 2;
    let half_ptr = base_ptr.add(half_count);

    for i in 0..=255 {
        let mut val: usize = 0;
        write_bytes(&mut val, i, size_of::<usize>() / size_of::<u8>());
        for j in 0..half_count {
            check_timeout(start_time, timeout_ms)?;
            write_volatile(base_ptr.add(j), val);
            write_volatile(half_ptr.add(j), val);
        }
        compare_regions(base_ptr, half_ptr, half_count, start_time, timeout_ms)?;
    }
    Ok(MemtestOutcome::Pass)
}

fn check_timeout(start_time: Instant, timeout_ms: usize) -> Result<(), MemtestError> {
    if start_time.elapsed() > Duration::from_millis(timeout_ms as u64) {
        Err(MemtestError::Timeout)
    } else {
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
