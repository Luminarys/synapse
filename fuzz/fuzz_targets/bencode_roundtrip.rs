#![no_main]
use libfuzzer_sys::fuzz_target;
use synapse_bencode as bencode;

fuzz_target!(|fuzz_data: &[u8]| {
    if let Ok(initial_bencode) = bencode::decode_buf(fuzz_data) {
        let mut buf = Vec::<u8>::new();
        initial_bencode.encode(&mut buf).unwrap();

        let roundtripped_bencode = bencode::decode_buf(&buf).unwrap();
        assert_eq!(initial_bencode, roundtripped_bencode);
    };
});
