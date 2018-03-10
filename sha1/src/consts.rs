#![cfg_attr(feature = "cargo-clippy", allow(unreadable_literal))]

pub const STATE_LEN: usize = 5;

pub const H: [u32; STATE_LEN] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
