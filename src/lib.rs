/*
Copyright (c) 2020 Todd Stellanova
LICENSE: BSD3 (see LICENSE file)
*/

#![cfg_attr(not(test), no_std)]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering::{SeqCst, Relaxed} };

pub const BUF_LEN: usize = 256;

pub struct Ringu {
    /// The actual buffer
    buf: [u8; BUF_LEN],

    /// The index at which the next byte should be read from the buffer
    /// This grows unbounded until it wraps, and is only masked into
    /// the inner buffer range when we access the array.
    read_idx: AtomicUsize,

    /// The index at which the next byte should be written to the buffer
    /// This grows unbounded until it wraps, and is only masked into
    /// the inner buffer range when we access the array.
    write_idx: AtomicUsize,

    /// a mutability lock
    mut_lock: AtomicBool,

    /// Used to implement a spin lock
    wait_counter: AtomicUsize,

}

impl Ringu {
    pub fn default() -> Self {
        Self {
            buf: [0; BUF_LEN],
            read_idx: AtomicUsize::new(0),
            write_idx: AtomicUsize::new(0),
            mut_lock: AtomicBool::new(false),
            wait_counter: AtomicUsize::new(0),
        }
    }

    fn lock_me(&mut self) {
        while self.mut_lock.compare_exchange(false, true, SeqCst, SeqCst).is_err() {
            //spin lock
            self.spinlock_up();
        }
    }

    fn unlock_me(&mut self) {
        while self.mut_lock.compare_exchange(true, false, SeqCst, SeqCst).is_err() {
            //spin lock
            self.spinlock_down();
        }
    }

    fn spinlock_up(&self) {
        self.wait_counter.fetch_add(1, Relaxed);
    }

    fn spinlock_down(&self) {
        self.wait_counter.fetch_sub(1, Relaxed);
    }

    pub fn available(&self) -> usize {
        self.write_idx.load(SeqCst) - self.read_idx.load(SeqCst)
    }

    pub fn is_full(&self) -> bool {
        self.available() == BUF_LEN
    }

    pub fn empty(&self) -> bool {
        self.write_idx.load(SeqCst) == self.read_idx.load(SeqCst)
    }

    pub fn push_one(&mut self, byte: u8) -> usize {
        self.lock_me();
        if !self.is_full() {
            //effectively this reserves space for the write
            let cur_write_idx = self.write_idx.fetch_add(1, SeqCst);
            // the actual write to the buffer
            self.buf[cur_write_idx & (BUF_LEN - 1)] = byte;
            self.unlock_me();
            1
        }
        else {
            self.unlock_me();
            0
        }
    }

    pub fn read_one(&mut self) -> (usize, u8) {
        self.lock_me();
        if !self.empty() {
            //"reserve" the read
            let cur_read_idx = self.read_idx.fetch_add(1, SeqCst);
            let byte = self.buf[cur_read_idx & (BUF_LEN - 1)];
            self.unlock_me();
            (1, byte)
        }
        else {
            self.unlock_me();
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


    #[test]
    fn multithread_write_read() {
        lazy_static!{
            static ref TOTAL_WRITE_COUNT:AtomicUsize = AtomicUsize::new(0);
            static ref BFFL: AtomicPtr<Ringu> = AtomicPtr::new(core::ptr::null_mut());
        };

        let mut bffl = Ringu::default();
        BFFL.store(&mut bffl, SeqCst);

        let inner_thread = thread::spawn(|| {
            for i in 0..100 {
                unsafe {
                    BFFL.load(SeqCst).as_mut().unwrap().push_one(i as u8);
                }
                TOTAL_WRITE_COUNT.fetch_add(1, SeqCst);
                if (i % 2) == 0 { thread::yield_now(); }
            }
        });

        let mut outer_thread_read_count = 0;
        for _ in 0..200 {
            let (nread, _b) = unsafe {
                BFFL.load(SeqCst).as_mut().unwrap().read_one()
            };
            outer_thread_read_count += nread;
            if nread == 0  {
                thread::yield_now();
            }
        }
        println!("outer_thread_read_count: {}", outer_thread_read_count);
        inner_thread.join().unwrap();

        assert_eq!(outer_thread_read_count, TOTAL_WRITE_COUNT.load(SeqCst));

    }

}
