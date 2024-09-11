use std::{
    ptr::{read_volatile, write_volatile},
    time::{Duration, Instant},
};

use rand::random;

use crate::Memtester;

#[derive(Debug)]
pub struct MemtestReport {
    test_type: MemtestType,
    outcome: Result<MemtestOutcome, MemtestError>,
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
    TestRandomValue,
}

impl Memtester {
    // TODO: test_own_address_alt
    // TODO: fix logging
    // TODO: if a more precise timeout is required, consider async/await
    // TODO: if time taken to run a test is too long,
    //       consider testing memory regions in smaller chunks
    pub(super) unsafe fn test_own_address(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        // println!("Starting test_own_address at {:?}", self.base_ptr);
        for i in 0..self.memcount {
            self.check_timeout(start_time)?;
            let ptr = self.base_ptr.add(i);
            write_volatile(ptr, ptr as usize);

            // if i % (self.memcount / 100) == 0 {
            //     print!("\x1B[2K\x1B[1G");
            //     print!(
            //         "Progress: {:.0}%",
            //         i as f64 / self.memcount as f64 * 100 as f64
            //     );
            //     io::stdout()
            //         .flush()
            //         .expect("Flushing stdout should be successful");
            // }
        }
        // println!("\nWrote own address to all addresses");
        // println!("Checking all addresses now:");

        for i in 0..self.memcount {
            self.check_timeout(start_time)?;
            let ptr = self.base_ptr.add(i);
            if read_volatile(ptr) != ptr as usize {
                println!("Error! {ptr:?} has unexpected value {:x}", *ptr);
                return Ok(MemtestOutcome::Fail(ptr as usize));
            }

            // if i % (self.memcount / 100) == 0 {
            //     print!("\x1B[2K\x1B[1G");
            //     print!(
            //         "Progress: {:.0}%",
            //         i as f64 / self.memcount as f64 * 100 as f64
            //     );
            //     io::stdout()
            //         .flush()
            //         .expect("Flushing stdout should be successful");
            // }
        }
        // println!("\nChecked all addresses");
        Ok(MemtestOutcome::Pass)
    }

    pub(super) unsafe fn test_random_value(
        &self,
        start_time: Instant,
    ) -> Result<MemtestOutcome, MemtestError> {
        let halfcount = self.memcount / 2;
        let half_ptr = self.base_ptr.add(halfcount);
        // println!("Starting test_own_address at {:?}", self.base_ptr);
        for i in 0..halfcount {
            self.check_timeout(start_time)?;
            let value = random();

            let ptr1 = self.base_ptr.add(i);
            let ptr2 = half_ptr.add(i);
            write_volatile(ptr1, value);
            write_volatile(ptr2, value);
        }

        for i in 0..halfcount {
            self.check_timeout(start_time)?;
            let ptr1 = self.base_ptr.add(i);
            let ptr2 = half_ptr.add(i);
            if read_volatile(ptr1) != read_volatile(ptr2) {
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
}

impl MemtestReport {
    pub(super) fn new(
        test_type: MemtestType,
        outcome: Result<MemtestOutcome, MemtestError>,
    ) -> MemtestReport {
        MemtestReport { test_type, outcome }
    }
}
