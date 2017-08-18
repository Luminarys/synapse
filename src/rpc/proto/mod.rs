extern crate rpc as rpc_lib;

pub mod ws;
pub mod message;
pub mod error;

pub use self::rpc_lib::resource;
pub use self::rpc_lib::criterion;
