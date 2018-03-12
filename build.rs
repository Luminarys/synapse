extern crate cc;

use std::env;

fn main() {
    let debug = env::var("DEBUG").unwrap() != "false";

    let fallocate_path = if cfg!(target_os = "linux") {
        "native/fallocate_linux.c"
    } else if cfg!(target_os = "macos") {
        "native/fallocate_darwin.c"
    } else if cfg!(target_family = "unix") {
        "native/fallocate_posix.c"
    } else {
        panic!("synapse can only be compiled on a POSIX platform!");
    };

    cc::Build::new()
        .file(fallocate_path)
        .opt_level(3)
        .debug(debug)
        .compile("fallocate");

    cc::Build::new()
        .file("native/mmap.c")
        .opt_level(3)
        .debug(debug)
        .warnings(false)
        .compile("mmap");
}
