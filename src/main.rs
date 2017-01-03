#[macro_use]
extern crate axon;
extern crate num_cpus;

use std::{process, thread, time};
use axon::Handle;

fn main() {
    let cores = num_cpus::get();
    let mut m = Handle::new(cores);
    loop {
        for ev in m.get_events() {
            // do something
        }
        thread::sleep(time::Duration::from_millis(1000));
    }
    // match m.run() {
    //     Ok(_) => process::exit(0),
    //     Err(_) => process::exit(1),
    // };
}
