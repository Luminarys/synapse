extern crate sha1;

use std::fmt::Write;
use sha1::Sha1;

fn hash_to_str(hash: &[u8]) -> String {
    let mut hash_str = String::new();
    for i in hash {
        write!(&mut hash_str, "{:02x}", i).unwrap();
    }
    hash_str
}

#[test]
fn test_simple() {
    let tests = [
            ("The quick brown fox jumps over the lazy dog",
             "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"),
            ("The quick brown fox jumps over the lazy cog",
             "de9f2c7fd25e1b3afad3e85a0bd17d9b100db4b3"),
            ("", "da39a3ee5e6b4b0d3255bfef95601890afd80709"),
            ("testing\n", "9801739daae44ec5293d4e1f53d3f4d2d426d91c"),
            ("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
             "025ecbd5d70f8fb3c5457cd96bab13fda305dc59"),
            ("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
             "4300320394f7ee239bcdce7d3b8bcee173a0cd5c"),
            ("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
             "cef734ba81a024479e09eb5a75b6ddae62e6abf1"),
        ];

    for &(s, ref h) in tests.iter() {
        let data = s.as_bytes();

        let res = hash_to_str(&Sha1::digest(data));
        assert_eq!(res.len(), h.len());
        assert_eq!(res, *h);
    }
}

#[test]
fn test_multiple_inputs() {
    let mut m = Sha1::new();
    m.input("The quick brown ".as_bytes());
    m.input("fox jumps over ".as_bytes());
    m.input("the lazy dog".as_bytes());
    let hh = hash_to_str(&m.result());

    let h = "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12";
    assert_eq!(hh.len(), h.len());
    assert_eq!(hh, &*h);
}

#[test]
fn test_sha1_loop() {
    let s = "The quick brown fox jumps over the lazy dog.";
    let n = 1000u64;

    for _ in 0..3 {
        let mut m = Sha1::new();
        for _ in 0..n {
            m.input(s.as_bytes());
        }
        assert_eq!(
            hash_to_str(&m.result()),
            "7ca27655f67fceaa78ed2e645a81c7f1d6e249d2"
        );
    }
}
