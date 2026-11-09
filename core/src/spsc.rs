//! Cola SPSC sin bloqueos de `u32` (un productor, un consumidor). Se usa para mandar de la
//! interfaz al hilo de audio las notas de prueba. Sin reservas de memoria.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

const CAP: usize = 64;

pub struct Spsc {
    buf: [AtomicU32; CAP],
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl Default for Spsc {
    fn default() -> Self {
        Spsc { buf: std::array::from_fn(|_| AtomicU32::new(0)), head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }
}

impl Spsc {
    /// Solo un hilo productor. Devuelve false si está llena.
    pub fn push(&self, v: u32) -> bool {
        let h = self.head.load(Ordering::Relaxed);
        if h.wrapping_sub(self.tail.load(Ordering::Acquire)) >= CAP {
            return false;
        }
        self.buf[h % CAP].store(v, Ordering::Relaxed);
        self.head.store(h.wrapping_add(1), Ordering::Release);
        true
    }

    /// Solo un hilo consumidor.
    pub fn pop(&self) -> Option<u32> {
        let t = self.tail.load(Ordering::Relaxed);
        if t == self.head.load(Ordering::Acquire) {
            return None;
        }
        let v = self.buf[t % CAP].load(Ordering::Relaxed);
        self.tail.store(t.wrapping_add(1), Ordering::Release);
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn fifo_and_full() {
        let q = Spsc::default();
        assert!(q.pop().is_none());
        for i in 0..CAP as u32 {
            assert!(q.push(i));
        }
        assert!(!q.push(999));
        for i in 0..CAP as u32 {
            assert_eq!(q.pop(), Some(i));
        }
        assert!(q.pop().is_none());
    }

    #[test]
    fn two_threads_keep_order() {
        let q = Arc::new(Spsc::default());
        let p = {
            let q = q.clone();
            std::thread::spawn(move || {
                for i in 0..50_000u32 {
                    while !q.push(i) {
                        std::thread::yield_now();
                    }
                }
            })
        };
        let mut next = 0u32;
        while next < 50_000 {
            if let Some(v) = q.pop() {
                assert_eq!(v, next);
                next += 1;
            }
        }
        p.join().unwrap();
    }
}
