use std::sync::Mutex;

pub struct Pool {
    items: Mutex<Vec<Box<[u8]>>>,
}

unsafe impl Sync for Pool { }
unsafe impl Send for Pool { }

impl Pool {
    pub fn new() -> Pool {
        Pool {
            items: Mutex::new(vec![vec![0u8; 16384].into_boxed_slice(); 100]),
        }
    }

    pub fn acquire(&self) -> Box<[u8]> {
        if let Some(b) = self.items.lock().unwrap().pop() {
            b
        } else {
            vec![0u8; 16384].into_boxed_slice()
        }
    }

    pub fn release(&self, b: Box<[u8]>) {
        self.items.lock().unwrap().push(b);
    }
}
