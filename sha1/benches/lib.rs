#![no_std]
#![feature(test)]
extern crate sha1;

macro_rules! bench {
    ($name: ident, $engine: path, $bs: expr) => {
        #[bench]
        fn $name(b: &mut Bencher) {
            let mut d = <$engine>::new();
            let data = [0; $bs];

            b.iter(|| {
                d.input(&data);
            });

            b.bytes = $bs;
        }
    };

    ($engine: path) => {
        extern crate test;

        use test::Bencher;

        bench!(bench1_10, $engine, 10);
        bench!(bench2_100, $engine, 100);
        bench!(bench3_1000, $engine, 1000);
        bench!(bench4_10000, $engine, 10000);
    };
}

bench!(sha1::Sha1);
