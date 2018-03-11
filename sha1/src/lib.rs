//! An implementation of the SHA-1 cryptographic hash algorithm.

//! To use this module, first create a `Sha1` object using the `Sha1` constructor,
//! then feed it an input message using the `input` or `input_str` methods,
//! which may be called any number of times; they will buffer the input until
//! there is enough to call the block algorithm.
//!
//! After the entire input has been fed to the hash read the result using
//! the `result` or `result_str` methods. The first will return bytes, and
//! the second will return a `String` object of the same bytes represented
//! in hexadecimal form.
//!
//! The `Sha1` object may be reused to create multiple hashes by calling
//! the `reset()` method. These traits are implemented by all hash digest
//! algorithms that implement the `Digest` trait. An example of use is:
//!
//! ```rust
//! use sha1::Sha1;
//!
//! // create a Sha1 object
//! let mut sh = Sha1::new();
//!
//! // write input message
//! sh.input(b"hello world");
//!
//! // read hash digest in the form of GenericArray which is in this case
//! // equivalent to [u8; 20]
//! let output = sh.result();
//! assert_eq!(output[..], [0x2a, 0xae, 0x6c, 0x35, 0xc9, 0x4f, 0xcf, 0xb4, 0x15, 0xdb,
//!                         0xe9, 0x5f, 0x40, 0x8b, 0x9c, 0xe9, 0x1e, 0xe8, 0x46, 0xed]);
//! ```
//!
//! # Mathematics
//!
//! The mathematics of the SHA-1 algorithm are quite interesting. In its
//! definition, The SHA-1 algorithm uses:
//!
//! * 1 binary operation on bit-arrays:
//!   * "exclusive or" (XOR)
//! * 2 binary operations on integers:
//!   * "addition" (ADD)
//!   * "rotate left" (ROL)
//! * 3 ternary operations on bit-arrays:
//!   * "choose" (CH)
//!   * "parity" (PAR)
//!   * "majority" (MAJ)
//!
//! Some of these functions are commonly found in all hash digest
//! algorithms, but some, like "parity" is only found in SHA-1.
#![no_std]
extern crate block_buffer;
extern crate byte_tools;

use byte_tools::write_u32v_be;
use block_buffer::BlockBuffer512;

mod consts;
use consts::{H, STATE_LEN};

#[link(name = "sha1-round")]
extern "C" {
    fn sha1_compress(state: &mut [u32; 5], block: &[u8; 64]);
}

/// Safe wrapper around assembly implementation of SHA-1 compression function
#[inline]
pub fn compress(state: &mut [u32; 5], block: &[u8; 64]) {
    unsafe {
        sha1_compress(state, block);
    }
}

/// Structure representing the state of a SHA-1 computation
#[derive(Clone)]
pub struct Sha1 {
    h: [u32; STATE_LEN],
    len: u64,
    buffer: BlockBuffer512,
}

impl Sha1 {
    pub fn new() -> Sha1 {
        Sha1 {
            h: H,
            len: 0u64,
            buffer: Default::default(),
        }
    }

    pub fn digest(data: &[u8]) -> [u8; 20] {
        let mut sha1 = Sha1::new();
        sha1.input(data);
        sha1.result()
    }

    pub fn input(&mut self, input: &[u8]) {
        // Assumes that `length_bits<<3` will not overflow
        self.len += input.len() as u64;
        let state = &mut self.h;
        self.buffer.input(input, |d| compress(state, d));
    }

    pub fn result(mut self) -> [u8; 20] {
        {
            let state = &mut self.h;
            let l = self.len << 3;
            // remove this mess by adding `len_padding_be` method
            let l = if cfg!(target_endian = "little") {
                l.to_be()
            } else {
                l.to_le()
            };
            self.buffer.len_padding(l, |d| compress(state, d));
        }
        let mut out = [0u8; 20];
        write_u32v_be(&mut out, &self.h);
        out
    }
}
