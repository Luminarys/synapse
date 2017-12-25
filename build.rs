extern crate cc;

fn main() {
    if cfg!(target_os = "linux") {
        cc::Build::new()
            .file("native/fallocate_linux.c")
            .compile("fallocate");
    } else if cfg!(target_os = "macos") {
        cc::Build::new()
            .file("native/fallocate_darwin.c")
            .compile("fallocate");
    } else if cfg!(target_family = "unix") {
        cc::Build::new()
            .file("native/fallocate_posix.c")
            .compile("fallocate");
    } else {
        panic!("synapse can only be compiled on a POSIX platform!");
    }
}
