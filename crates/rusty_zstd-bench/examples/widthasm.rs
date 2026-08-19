//! What does a fixed-width copy_nonoverlapping actually COMPILE to?
//!
//! The store-traffic model priced the fast path in BYTES. If 8, 16 and 32 bytes
//! all lower to one store instruction touching one cache line, then bytes were
//! never the cost and the model was measuring a quantity the machine does not
//! charge for.
#[inline(never)]
pub fn cp8(d: &mut Vec<u8>, s: &[u8], from: usize, n: usize) {
    let len = d.len();
    unsafe { core::ptr::copy_nonoverlapping(s.as_ptr().add(from), d.as_mut_ptr().add(len), 8); d.set_len(len + n); }
}
#[inline(never)]
pub fn cp16(d: &mut Vec<u8>, s: &[u8], from: usize, n: usize) {
    let len = d.len();
    unsafe { core::ptr::copy_nonoverlapping(s.as_ptr().add(from), d.as_mut_ptr().add(len), 16); d.set_len(len + n); }
}
#[inline(never)]
pub fn cp32(d: &mut Vec<u8>, s: &[u8], from: usize, n: usize) {
    let len = d.len();
    unsafe { core::ptr::copy_nonoverlapping(s.as_ptr().add(from), d.as_mut_ptr().add(len), 32); d.set_len(len + n); }
}
#[inline(never)]
pub fn cp64(d: &mut Vec<u8>, s: &[u8], from: usize, n: usize) {
    let len = d.len();
    unsafe { core::ptr::copy_nonoverlapping(s.as_ptr().add(from), d.as_mut_ptr().add(len), 64); d.set_len(len + n); }
}
fn main() {
    let s: Vec<u8> = (0..4096u32).map(|i| i as u8).collect();
    let mut d: Vec<u8> = Vec::with_capacity(1 << 16);
    for f in [cp8 as fn(&mut Vec<u8>, &[u8], usize, usize), cp16, cp32, cp64] {
        d.clear();
        f(&mut d, &s, 0, 4);
    }
    println!("len {}", d.len());
}
