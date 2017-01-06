use std::io::{self, Read};
use message::Message;

pub struct Reader {
}

impl Reader {
    pub fn new() {
        Reader {
        
        }
    }

    pub fn readable(&mut self) -> io::Result<Option<Message>> {
        Ok(None)
    }
}
