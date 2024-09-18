use std::{
    mem::size_of,
    ptr::{read_volatile, write_bytes, write_volatile},
    time::{Duration, Instant},
};

use rand::random;

use crate::Memtester;

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

impl Memtester {
    // TODO: Logging?
    // TODO: if a more precise timeout is required, consider async
    // TODO: if time taken to run a test is too long,
    //       consider testing memory regions in smaller chunks

    // TODO:
    // According to memtester, this needs to be run several times,
    // and with alternating complements of aaddress
    pub(super) unsafe fn test_own_address(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        for i in 0..self.mem_usize_count {
            self.check_timeout(start_time)?;
            let ptr = self.base_ptr.add(i);
            write_volatile(ptr, ptr as usize);
        }

        for i in 0..self.mem_usize_count {
            self.check_timeout(start_time)?;
            let ptr = self.base_ptr.add(i);
            if read_volatile(ptr) != ptr as usize {
                return Ok(MemtestOutcome::Fail(ptr as usize));
            }
        }
        Ok(MemtestOutcome::Pass)
    }

    pub(super) unsafe fn test_random_val(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        let half_memcount = self.mem_usize_count / 2;
        let half_ptr = self.base_ptr.add(half_memcount);
        for i in 0..half_memcount {
            self.check_timeout(start_time)?;
            let value = random();
            write_volatile(self.base_ptr.add(i), value);
            write_volatile(half_ptr.add(i), value);
        }
        self.compare_regions(start_time, self.base_ptr, half_ptr, half_memcount)
    }

    pub(super) unsafe fn test_xor(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        self.test_two_regions(start_time, Memtester::write_xor)
    }

    pub(super) unsafe fn test_sub(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        self.test_two_regions(start_time, Memtester::write_sub)
    }

    pub(super) unsafe fn test_mul(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        self.test_two_regions(start_time, Memtester::write_mul)
    }

    pub(super) unsafe fn test_div(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        self.test_two_regions(start_time, Memtester::write_div)
    }

    pub(super) unsafe fn test_or(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        self.test_two_regions(start_time, Memtester::write_or)
    }

    pub(super) unsafe fn test_and(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        self.test_two_regions(start_time, Memtester::write_and)
    }

    unsafe fn test_two_regions<F>(
        &self,
        start_time: Instant,
        write_val: F,
    ) -> Result<MemtestOutcome, MemtestError>
    where
        F: Fn(*mut usize, *mut usize, usize),
    {
        write_bytes(self.base_ptr, 0xff, self.mem_usize_count);

        let half_memcount = self.mem_usize_count / 2;
        let half_ptr = self.base_ptr.add(half_memcount);

        for i in 0..half_memcount {
            self.check_timeout(start_time)?;
            write_val(self.base_ptr.add(i), half_ptr.add(i), random());
        }

        self.compare_regions(start_time, self.base_ptr, half_ptr, half_memcount)
    }

    unsafe fn compare_regions(
        &self,
        start_time: Instant,
        ptr1: *const usize,
        ptr2: *const usize,
        count: usize,
    ) -> Result<MemtestOutcome, MemtestError> {
        for i in 0..count {
            self.check_timeout(start_time)?;
            if read_volatile(ptr1.add(i)) != read_volatile(ptr2.add(i)) {
                return Ok(MemtestOutcome::Fail(ptr1 as usize));
            }
        }
        Ok(MemtestOutcome::Pass)
    }

    fn check_timeout(&self, start_time: Instant) -> Result<(), MemtestError> {
        if start_time.elapsed() > Duration::from_millis(self.timeout_ms as u64) {
            Err(MemtestError::Timeout)
        } else {
            Ok(())
        }
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

    pub(super) unsafe fn test_seq_inc(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        let half_memcount = self.mem_usize_count / 2;
        let half_ptr = self.base_ptr.add(half_memcount);

        let value: usize = random();
        for i in 0..half_memcount {
            self.check_timeout(start_time)?;
            write_volatile(self.base_ptr.add(i), value.wrapping_add(i));
            write_volatile(half_ptr.add(i), value.wrapping_add(i));
        }
        self.compare_regions(start_time, self.base_ptr, half_ptr, half_memcount)
    }

    pub(super) unsafe fn test_solid_bits(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        let half_memcount = self.mem_usize_count / 2;
        let half_ptr = self.base_ptr.add(half_memcount);

        for i in 0..64 {
            let val = if i % 2 == 0 { !0 } else { 0 };
            for j in 0..half_memcount {
                self.check_timeout(start_time)?;
                let val = if j % 2 == 0 { val } else { !val };
                write_volatile(self.base_ptr.add(j), val);
                write_volatile(half_ptr.add(j), val);
            }
            self.compare_regions(start_time, self.base_ptr, half_ptr, half_memcount)?;
        }
        Ok(MemtestOutcome::Pass)
    }

    pub(super) unsafe fn test_checkerboard(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        const CHECKER_BOARD: usize = 0x5555555555555555;
        let half_memcount = self.mem_usize_count / 2;
        let half_ptr = self.base_ptr.add(half_memcount);

        for i in 0..64 {
            let val = if i % 2 == 0 {
                CHECKER_BOARD
            } else {
                !CHECKER_BOARD
            };
            for j in 0..half_memcount {
                self.check_timeout(start_time)?;
                let val = if j % 2 == 0 { val } else { !val };
                write_volatile(self.base_ptr.add(j), val);
                write_volatile(half_ptr.add(j), val);
            }
            self.compare_regions(start_time, self.base_ptr, half_ptr, half_memcount)?;
        }
        Ok(MemtestOutcome::Pass)
    }

    pub(super) unsafe fn test_block_seq(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        let half_memcount = self.mem_usize_count / 2;
        let half_ptr = self.base_ptr.add(half_memcount);

        for i in 0..=255 {
            let mut val: usize = 0;
            write_bytes(&mut val, i, size_of::<usize>() / size_of::<u8>());
            for j in 0..half_memcount {
                self.check_timeout(start_time)?;
                write_volatile(self.base_ptr.add(j), val);
                write_volatile(half_ptr.add(j), val);
            }
            self.compare_regions(start_time, self.base_ptr, half_ptr, half_memcount)?;
        }
        Ok(MemtestOutcome::Pass)
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
