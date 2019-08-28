extern crate chrono;
extern crate regex;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate url;
extern crate url_serde;

pub mod criterion;
pub mod message;
pub mod resource;

pub const MAJOR_VERSION: u16 = 0;
pub const MINOR_VERSION: u16 = 1;
