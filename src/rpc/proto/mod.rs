extern crate synapse_rpc as rpc_lib;

pub mod ws;
pub mod error;

pub use self::rpc_lib::resource;
pub use self::rpc_lib::criterion;
pub use self::rpc_lib::message;
