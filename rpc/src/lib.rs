#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate regex;
extern crate chrono;

pub mod resource;
pub mod criterion;
pub mod message;

pub const MAJOR_VERSION: u16 = 0;
pub const MINOR_VERSION: u16 = 1;
