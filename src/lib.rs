/*
Copyright (c) 2022 Todd Stellanova
LICENSE: BSD3 (see LICENSE file)
*/

#![cfg_attr(not(test), no_std)]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering };

// pub const BUF_LEN: usize = 256;

pub type SpinFunc = fn() ;

pub struct Ringu<const N: usize> {
    /// The actual buffer
    buf: [u8; N],

    /// The index at which the next byte should be read from the buffer
    /// This grows unbounded until it wraps, and is only masked into
    /// the inner buffer range when we access the array.
    read_idx: AtomicUsize,

    /// The index at which the next byte should be written to the buffer
    /// This grows unbounded until it wraps, and is only masked into
    /// the inner buffer range when we access the array.
    write_idx: AtomicUsize,

    /// A mutability lock
    mut_lock: AtomicBool,

    /// Optional user-overridden spin lock function
    spin_func: SpinFunc,

    /// tracking bytes read
    read_count: AtomicUsize,
}

impl<const N: usize> Ringu<N> {
    pub fn default() -> Self {
        Self {
            buf: [0; N],
            read_idx: AtomicUsize::new(0),
            write_idx: AtomicUsize::new(0),
            mut_lock: AtomicBool::new(false),
            spin_func: Self::spinlock,
            read_count: AtomicUsize::new(0),
        }
    }

    /// Provide a custom spin function that will be called when we're trying to lock this struct
    pub fn new_with_spin(spin: SpinFunc) -> Self {
        Self {
            buf: [0; N],
            read_idx: AtomicUsize::new(0),
            write_idx: AtomicUsize::new(0),
            mut_lock: AtomicBool::new(false),
            spin_func: spin,
            read_count: AtomicUsize::new(0),
        }
    }

    fn lock_me(&mut self) {
        while self.mut_lock.compare_and_swap(false, true, Ordering::Acquire) != false {
            while self.mut_lock.load(Ordering::Relaxed) {
                (self.spin_func)();
            }
        }
    }

    fn unlock_me(&mut self) {
        self.mut_lock.compare_and_swap(true, false, Ordering::Acquire);
    }

    fn spinlock() {
        core::hint::spin_loop();
    }


    /// How much data is available to be read?
    pub fn available(&self) -> usize {
        let write = self.write_idx.load(Ordering::SeqCst);
        let read = self.read_idx.load(Ordering::SeqCst);
        let avail = write.wrapping_sub(read);
        let read_count = self.read_count.load(Ordering::Relaxed);
        assert!(avail <= N, "avail: {} write: {} read: {} count: {}", avail, write, read, read_count);
        avail
    }

    /// Is the buffer full?
    pub fn full(&self) -> bool {
        self.available() == N
    }

    /// Is the buffer empty?
    pub fn empty(&self) -> bool {
        self.write_idx.load(Ordering::SeqCst) == self.read_idx.load(Ordering::SeqCst)
    }

    /// At the moment, how much vacant space remains in the buffer?
    pub fn vacant(&self) -> usize {
        N - self.available()
    }

    fn lock_if_not_full(&mut self) -> bool {
        if !self.full() {
            self.lock_me();
            true
        }
        else {
            false
        }
    }

    fn lock_if_not_empty(&mut self) -> bool {
        if !self.empty() {
            self.lock_me();
            true
        }
        else {
            false
        }
    }

    /// Push one byte into the buffer
    /// Returns the number of bytes actually pushed (zero or one)
    pub fn push_one(&mut self, byte: u8) -> usize {
        if self.lock_if_not_full() {
            //effectively this reserves space for the write
            let cur_write_idx = self.write_idx.fetch_add(1, Ordering::SeqCst);
            self.buf[cur_write_idx & (N - 1)] = byte;
            self.unlock_me();
            1
        }
        else {
            0
        }
    }

    /// Read one byte from the buffer
    /// Returns the number of bytes actually read (zero or one)
    /// and the byte read (if any)
    pub fn read_one(&mut self) -> (usize, u8) {
        if self.lock_if_not_empty() {
            //"reserve" the read
            self.read_count.fetch_add(1, Ordering::Relaxed);
            let cur_read_idx = self.read_idx.fetch_add(1, Ordering::SeqCst);
            let byte = self.buf[cur_read_idx & (N - 1)];
            self.unlock_me();
            (1, byte)
        }
        else {
            (0, 0)
        }
    }

}


#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use lazy_static::lazy_static;
    use core::sync::atomic::{AtomicUsize, AtomicPtr, Ordering::SeqCst};

    // used for testing custom spin func
    fn fake_spin() {
        core::hint::spin_loop();
    }

    /// Test for eventual consistency (number of writes == number reads)
    #[test]
    fn multithread_write_read() {
        lazy_static!{
            static ref TOTAL_WRITE_COUNT:AtomicUsize = AtomicUsize::new(0);
            static ref BLOCKED_WRITE_COUNT:AtomicUsize = AtomicUsize::new(0);
            static ref BFFL: AtomicPtr<Ringu<256>> = AtomicPtr::default();
        };

        const MAX_WRITE_COUNT: usize = 512;
        const MAX_READ_COUNT: usize = MAX_WRITE_COUNT * 3;

        let mut bffl = Ringu::new_with_spin(fake_spin);
        BFFL.store(&mut bffl, SeqCst);

        let inner_thread = thread::spawn(|| {
            //write more than BUF_LEN size
            for i in 0..MAX_WRITE_COUNT {
                let n_written = unsafe {
                    BFFL.load(SeqCst).as_mut().unwrap().push_one((i % 256) as u8 )
                };
                TOTAL_WRITE_COUNT.fetch_add(n_written, SeqCst);
                if 0 == n_written {
                    BLOCKED_WRITE_COUNT.fetch_add(1, SeqCst);
                }
                if (0 == n_written) ||  ((i % 2) == 0) {
                    thread::yield_now();
                }
            }
        });

        let mut read_attempts = 0;
        let mut outer_read_count = 0;
        let mut prior_read_val: u8 = 255;
        for _ in 0..MAX_READ_COUNT {
            let (nread, cur_val) =
                unsafe {
                    BFFL.load(SeqCst).as_mut().unwrap().read_one()
                };
            read_attempts += 1;
            outer_read_count += nread;
            if nread == 0  {
                thread::yield_now();
            }
            else {
                //verify that we receive the bytes in sequence
                assert!(cur_val.wrapping_sub(prior_read_val) == 1);
                prior_read_val = cur_val;
            }
        }

        println!("read_attempts: {} outer_read_count: {}", read_attempts, outer_read_count);
        inner_thread.join().unwrap();

        println!("blocked writes: {}", BLOCKED_WRITE_COUNT.load(SeqCst));
        assert_eq!(outer_read_count, TOTAL_WRITE_COUNT.load(SeqCst));

        assert_eq!(0, BLOCKED_WRITE_COUNT.load(SeqCst));
    }

}
