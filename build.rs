extern crate cc;

fn main() {
    if cfg!(target_os = "linux") {
        cc::Build::new()
            .file("native/fallocate_linux.c")
            .opt_level(3)
            .compile("fallocate");
    } else if cfg!(target_os = "macos") {
        cc::Build::new()
            .file("native/fallocate_darwin.c")
            .opt_level(3)
            .compile("fallocate");
    } else if cfg!(target_family = "unix") {
        cc::Build::new()
            .file("native/fallocate_posix.c")
            .opt_level(3)
            .compile("fallocate");
    } else {
        panic!("synapse can only be compiled on a POSIX platform!");
    }

    cc::Build::new()
        .file("native/mmap.c")
        .opt_level(3)
        .warnings(false)
        .compile("mmap");
}
