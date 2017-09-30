#![no_std]

mod simd;
use simd::*;

use core::cmp;
use core::fmt;
use core::mem;

/// The length of a SHA1 digest in bytes
pub const DIGEST_LENGTH: usize = 20;

/// Represents a Sha1 hash object in memory.
#[derive(Clone)]
pub struct Sha1 {
    state: Sha1State,
    blocks: Blocks,
    len: u64,
}

struct Blocks {
    len: u32,
    block: [u8; 64],
}

#[derive(Copy, Clone)]
struct Sha1State {
    state: [u32; 5],
}

/// Digest generated from a `Sha1` instance.
///
/// A digest can be formatted to view the digest as a hex string, or the bytes
/// can be extracted for later processing.
pub struct Digest {
    data: Sha1State,
}

const DEFAULT_STATE: Sha1State =
    Sha1State { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0] };

#[inline(always)]
fn as_block(input: &[u8]) -> &[u8; 64] {
    unsafe {
        assert!(input.len() == 64);
        let arr: &[u8; 64] = mem::transmute(input.as_ptr());
        arr
    }
}

impl Sha1 {
    /// Creates an fresh sha1 hash object.
    #[cfg_attr(debug_assertions, inline(never))]
    #[cfg_attr(not(debug_assertions), inline(always))]
    pub fn new() -> Sha1 {
        Sha1 {
            state: DEFAULT_STATE,
            len: 0,
            blocks: Blocks {
                len: 0,
                block: [0; 64],
            },
        }
    }

    /// Resets the hash object to it's initial state.
    pub fn reset(&mut self) {
        self.state = DEFAULT_STATE;
        self.len = 0;
        self.blocks.len = 0;
    }

    /// Update hash with input data.
    #[cfg_attr(debug_assertions, inline(never))]
    #[cfg_attr(not(debug_assertions), inline(always))]
    pub fn update(&mut self, data: &[u8]) {
        let len = &mut self.len;
        let state = &mut self.state;
        self.blocks.input(data, |block| {
            *len += block.len() as u64;
            state.process(block);
        })
    }

    #[cfg_attr(debug_assertions, inline(never))]
    #[cfg_attr(not(debug_assertions), inline(always))]
    pub fn finish(&self) -> [u8; DIGEST_LENGTH] {
        self.digest().bytes()
    }

    /// Retrieve digest result.
    #[cfg_attr(debug_assertions, inline(never))]
    #[cfg_attr(not(debug_assertions), inline(always))]
    pub fn digest(&self) -> Digest {
        let mut state = self.state;
        let bits = (self.len + (self.blocks.len as u64)) * 8;
        let extra = [
            (bits >> 56) as u8,
            (bits >> 48) as u8,
            (bits >> 40) as u8,
            (bits >> 32) as u8,
            (bits >> 24) as u8,
            (bits >> 16) as u8,
            (bits >> 8) as u8,
            (bits >> 0) as u8,
        ];
        let mut last = [0; 128];
        let blocklen = self.blocks.len as usize;
        last[..blocklen].clone_from_slice(&self.blocks.block[..blocklen]);
        last[blocklen] = 0x80;

        if blocklen < 56 {
            last[56..64].clone_from_slice(&extra);
            state.process(as_block(&last[0..64]));
        } else {
            last[120..128].clone_from_slice(&extra);
            state.process(as_block(&last[0..64]));
            state.process(as_block(&last[64..128]));
        }

        Digest { data: state }
    }
}

impl Digest {
    /// Returns the 160 bit (20 byte) digest as a byte array.
    #[cfg_attr(debug_assertions, inline(never))]
    #[cfg_attr(not(debug_assertions), inline(always))]
    pub fn bytes(&self) -> [u8; DIGEST_LENGTH] {
        [
            (self.data.state[0] >> 24) as u8,
            (self.data.state[0] >> 16) as u8,
            (self.data.state[0] >> 8) as u8,
            (self.data.state[0] >> 0) as u8,
            (self.data.state[1] >> 24) as u8,
            (self.data.state[1] >> 16) as u8,
            (self.data.state[1] >> 8) as u8,
            (self.data.state[1] >> 0) as u8,
            (self.data.state[2] >> 24) as u8,
            (self.data.state[2] >> 16) as u8,
            (self.data.state[2] >> 8) as u8,
            (self.data.state[2] >> 0) as u8,
            (self.data.state[3] >> 24) as u8,
            (self.data.state[3] >> 16) as u8,
            (self.data.state[3] >> 8) as u8,
            (self.data.state[3] >> 0) as u8,
            (self.data.state[4] >> 24) as u8,
            (self.data.state[4] >> 16) as u8,
            (self.data.state[4] >> 8) as u8,
            (self.data.state[4] >> 0) as u8,
        ]
    }
}

impl Blocks {
    fn input<F>(&mut self, mut input: &[u8], mut f: F)
    where
        F: FnMut(&[u8; 64]),
    {
        if self.len > 0 {
            let len = self.len as usize;
            let amt = cmp::min(input.len(), self.block.len() - len);
            self.block[len..len + amt].clone_from_slice(&input[..amt]);
            if len + amt == self.block.len() {
                f(&self.block);
                self.len = 0;
                input = &input[amt..];
            } else {
                self.len += amt as u32;
                return;
            }
        }
        assert_eq!(self.len, 0);
        for chunk in input.chunks(64) {
            if chunk.len() == 64 {
                f(as_block(chunk))
            } else {
                self.block[..chunk.len()].clone_from_slice(chunk);
                self.len = chunk.len() as u32;
            }
        }
    }
}

// Round key constants
const K0: u32 = 0x5A827999u32;
const K1: u32 = 0x6ED9EBA1u32;
const K2: u32 = 0x8F1BBCDCu32;
const K3: u32 = 0xCA62C1D6u32;

/// Not an intrinsic, but gets the first element of a vector.
#[inline]
pub fn sha1_first(w0: u32x4) -> u32 {
    w0.0
}

/// Not an intrinsic, but adds a word to the first element of a vector.
#[inline]
pub fn sha1_first_add(e: u32, w0: u32x4) -> u32x4 {
    let u32x4(a, b, c, d) = w0;
    u32x4(e.wrapping_add(a), b, c, d)
}

/// Emulates `llvm.x86.sha1msg1` intrinsic.
fn sha1msg1(a: u32x4, b: u32x4) -> u32x4 {
    let u32x4(_, _, w2, w3) = a;
    let u32x4(w4, w5, _, _) = b;
    a ^ u32x4(w2, w3, w4, w5)
}

/// Emulates `llvm.x86.sha1msg2` intrinsic.
fn sha1msg2(a: u32x4, b: u32x4) -> u32x4 {
    let u32x4(x0, x1, x2, x3) = a;
    let u32x4(_, w13, w14, w15) = b;

    let w16 = (x0 ^ w13).rotate_left(1);
    let w17 = (x1 ^ w14).rotate_left(1);
    let w18 = (x2 ^ w15).rotate_left(1);
    let w19 = (x3 ^ w16).rotate_left(1);

    u32x4(w16, w17, w18, w19)
}

/// Performs 4 rounds of the message schedule update.
pub fn sha1_schedule_x4(v0: u32x4, v1: u32x4, v2: u32x4, v3: u32x4) -> u32x4 {
    sha1msg2(sha1msg1(v0, v1) ^ v2, v3)
}

/// Emulates `llvm.x86.sha1nexte` intrinsic.
#[inline]
pub fn sha1_first_half(abcd: u32x4, msg: u32x4) -> u32x4 {
    sha1_first_add(sha1_first(abcd).rotate_left(30), msg)
}

/// Emulates `llvm.x86.sha1rnds4` intrinsic.
/// Performs 4 rounds of the message block digest.
pub fn sha1_digest_round_x4(abcd: u32x4, work: u32x4, i: i8) -> u32x4 {
    const K0V: u32x4 = u32x4(K0, K0, K0, K0);
    const K1V: u32x4 = u32x4(K1, K1, K1, K1);
    const K2V: u32x4 = u32x4(K2, K2, K2, K2);
    const K3V: u32x4 = u32x4(K3, K3, K3, K3);

    match i {
        0 => sha1rnds4c(abcd, work + K0V),
        1 => sha1rnds4p(abcd, work + K1V),
        2 => sha1rnds4m(abcd, work + K2V),
        3 => sha1rnds4p(abcd, work + K3V),
        _ => panic!("unknown icosaround index"),
    }
}

/// Not an intrinsic, but helps emulate `llvm.x86.sha1rnds4` intrinsic.
fn sha1rnds4c(abcd: u32x4, msg: u32x4) -> u32x4 {
    let u32x4(mut a, mut b, mut c, mut d) = abcd;
    let u32x4(t, u, v, w) = msg;
    let mut e = 0u32;

    macro_rules! bool3ary_202 {
        ($a:expr, $b:expr, $c:expr) => (($c ^ ($a & ($b ^ $c))))
    } // Choose, MD5F, SHA1C

    e = e.wrapping_add(a.rotate_left(5))
        .wrapping_add(bool3ary_202!(b, c, d))
        .wrapping_add(t);
    b = b.rotate_left(30);

    d = d.wrapping_add(e.rotate_left(5))
        .wrapping_add(bool3ary_202!(a, b, c))
        .wrapping_add(u);
    a = a.rotate_left(30);

    c = c.wrapping_add(d.rotate_left(5))
        .wrapping_add(bool3ary_202!(e, a, b))
        .wrapping_add(v);
    e = e.rotate_left(30);

    b = b.wrapping_add(c.rotate_left(5))
        .wrapping_add(bool3ary_202!(d, e, a))
        .wrapping_add(w);
    d = d.rotate_left(30);

    u32x4(b, c, d, e)
}

/// Not an intrinsic, but helps emulate `llvm.x86.sha1rnds4` intrinsic.
fn sha1rnds4p(abcd: u32x4, msg: u32x4) -> u32x4 {
    let u32x4(mut a, mut b, mut c, mut d) = abcd;
    let u32x4(t, u, v, w) = msg;
    let mut e = 0u32;

    macro_rules! bool3ary_150 {
        ($a:expr, $b:expr, $c:expr) => (($a ^ $b ^ $c))
    } // Parity, XOR, MD5H, SHA1P

    e = e.wrapping_add(a.rotate_left(5))
        .wrapping_add(bool3ary_150!(b, c, d))
        .wrapping_add(t);
    b = b.rotate_left(30);

    d = d.wrapping_add(e.rotate_left(5))
        .wrapping_add(bool3ary_150!(a, b, c))
        .wrapping_add(u);
    a = a.rotate_left(30);

    c = c.wrapping_add(d.rotate_left(5))
        .wrapping_add(bool3ary_150!(e, a, b))
        .wrapping_add(v);
    e = e.rotate_left(30);

    b = b.wrapping_add(c.rotate_left(5))
        .wrapping_add(bool3ary_150!(d, e, a))
        .wrapping_add(w);
    d = d.rotate_left(30);

    u32x4(b, c, d, e)
}

/// Not an intrinsic, but helps emulate `llvm.x86.sha1rnds4` intrinsic.
fn sha1rnds4m(abcd: u32x4, msg: u32x4) -> u32x4 {
    let u32x4(mut a, mut b, mut c, mut d) = abcd;
    let u32x4(t, u, v, w) = msg;
    let mut e = 0u32;

    macro_rules! bool3ary_232 {
        ($a:expr, $b:expr, $c:expr) => (($a & $b) ^ ($a & $c) ^ ($b & $c))
    } // Majority, SHA1M

    e = e.wrapping_add(a.rotate_left(5))
        .wrapping_add(bool3ary_232!(b, c, d))
        .wrapping_add(t);
    b = b.rotate_left(30);

    d = d.wrapping_add(e.rotate_left(5))
        .wrapping_add(bool3ary_232!(a, b, c))
        .wrapping_add(u);
    a = a.rotate_left(30);

    c = c.wrapping_add(d.rotate_left(5))
        .wrapping_add(bool3ary_232!(e, a, b))
        .wrapping_add(v);
    e = e.rotate_left(30);

    b = b.wrapping_add(c.rotate_left(5))
        .wrapping_add(bool3ary_232!(d, e, a))
        .wrapping_add(w);
    d = d.rotate_left(30);

    u32x4(b, c, d, e)
}

impl Sha1State {
    fn process(&mut self, block: &[u8; 64]) {
        let mut words = [0u32; 16];
        for i in 0..16 {
            let off = i * 4;
            words[i] = (block[off + 3] as u32) | ((block[off + 2] as u32) << 8) |
                ((block[off + 1] as u32) << 16) |
                ((block[off] as u32) << 24);
        }
        macro_rules! schedule {
            ($v0:expr, $v1:expr, $v2:expr, $v3:expr) => (
                sha1msg2(sha1msg1($v0, $v1) ^ $v2, $v3)
            )
        }

        macro_rules! rounds4 {
            ($h0:ident, $h1:ident, $wk:expr, $i:expr) => (
                sha1_digest_round_x4($h0, sha1_first_half($h1, $wk), $i)
            )
        }

        // Rounds 0..20
        let mut h0 = u32x4(self.state[0], self.state[1], self.state[2], self.state[3]);
        let mut w0 = u32x4(words[0], words[1], words[2], words[3]);
        let mut h1 = sha1_digest_round_x4(h0, sha1_first_add(self.state[4], w0), 0);
        let mut w1 = u32x4(words[4], words[5], words[6], words[7]);
        h0 = rounds4!(h1, h0, w1, 0);
        let mut w2 = u32x4(words[8], words[9], words[10], words[11]);
        h1 = rounds4!(h0, h1, w2, 0);
        let mut w3 = u32x4(words[12], words[13], words[14], words[15]);
        h0 = rounds4!(h1, h0, w3, 0);
        let mut w4 = schedule!(w0, w1, w2, w3);
        h1 = rounds4!(h0, h1, w4, 0);

        // Rounds 20..40
        w0 = schedule!(w1, w2, w3, w4);
        h0 = rounds4!(h1, h0, w0, 1);
        w1 = schedule!(w2, w3, w4, w0);
        h1 = rounds4!(h0, h1, w1, 1);
        w2 = schedule!(w3, w4, w0, w1);
        h0 = rounds4!(h1, h0, w2, 1);
        w3 = schedule!(w4, w0, w1, w2);
        h1 = rounds4!(h0, h1, w3, 1);
        w4 = schedule!(w0, w1, w2, w3);
        h0 = rounds4!(h1, h0, w4, 1);

        // Rounds 40..60
        w0 = schedule!(w1, w2, w3, w4);
        h1 = rounds4!(h0, h1, w0, 2);
        w1 = schedule!(w2, w3, w4, w0);
        h0 = rounds4!(h1, h0, w1, 2);
        w2 = schedule!(w3, w4, w0, w1);
        h1 = rounds4!(h0, h1, w2, 2);
        w3 = schedule!(w4, w0, w1, w2);
        h0 = rounds4!(h1, h0, w3, 2);
        w4 = schedule!(w0, w1, w2, w3);
        h1 = rounds4!(h0, h1, w4, 2);

        // Rounds 60..80
        w0 = schedule!(w1, w2, w3, w4);
        h0 = rounds4!(h1, h0, w0, 3);
        w1 = schedule!(w2, w3, w4, w0);
        h1 = rounds4!(h0, h1, w1, 3);
        w2 = schedule!(w3, w4, w0, w1);
        h0 = rounds4!(h1, h0, w2, 3);
        w3 = schedule!(w4, w0, w1, w2);
        h1 = rounds4!(h0, h1, w3, 3);
        w4 = schedule!(w0, w1, w2, w3);
        h0 = rounds4!(h1, h0, w4, 3);

        let e = sha1_first(h1).rotate_left(30);
        let u32x4(a, b, c, d) = h0;

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
    }
}

impl Clone for Blocks {
    fn clone(&self) -> Blocks {
        Blocks { ..*self }
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for i in self.data.state.iter() {
            try!(write!(f, "{:08x}", i));
        }
        Ok(())
    }
}

#[cfg_attr(rustfmt, rustfmt_skip)]
#[cfg(test)]
mod tests {
    extern crate std;

    use self::std::prelude::v1::*;

    use Sha1;

    #[test]
    fn test_simple() {
        let mut m = Sha1::new();

        let tests = [
            ("The quick brown fox jumps over the lazy dog",
             "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"),
            ("The quick brown fox jumps over the lazy cog",
             "de9f2c7fd25e1b3afad3e85a0bd17d9b100db4b3"),
            ("", "da39a3ee5e6b4b0d3255bfef95601890afd80709"),
            ("testing\n", "9801739daae44ec5293d4e1f53d3f4d2d426d91c"),
            ("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
             "025ecbd5d70f8fb3c5457cd96bab13fda305dc59"),
            ("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
             "cef734ba81a024479e09eb5a75b6ddae62e6abf1"),
        ];

        for &(s, ref h) in tests.iter() {
            let data = s.as_bytes();

            m.reset();
            m.update(data);
            let hh = m.digest().to_string();

            assert_eq!(hh.len(), h.len());
            assert_eq!(hh, *h);
        }
    }

    #[test]
    fn test_multiple_updates() {
        let mut m = Sha1::new();

        m.reset();
        m.update("The quick brown ".as_bytes());
        m.update("fox jumps over ".as_bytes());
        m.update("the lazy dog".as_bytes());
        let hh = m.digest().to_string();


        let h = "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12";
        assert_eq!(hh.len(), h.len());
        assert_eq!(hh, &*h);
    }

    #[test]
    fn test_sha1_loop() {
        let mut m = Sha1::new();
        let s = "The quick brown fox jumps over the lazy dog.";
        let n = 1000u64;

        for _ in 0..3 {
            m.reset();
            for _ in 0..n {
                m.update(s.as_bytes());
            }
            assert_eq!(m.digest().to_string(),
                       "7ca27655f67fceaa78ed2e645a81c7f1d6e249d2");
        }
    }
}
