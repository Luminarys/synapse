use std::cell::UnsafeCell;
use std::rc::Rc;
use amy::Registrar;
use std::collections::HashSet;

/// Creates a throttler from which sub throttles may be created.
/// Note that all created throttle's have a lifetime tied to the
/// throttler. This invariant must be maintained or undefined
/// behaviour will occur.
pub struct Throttler {
    id: usize,
    fid: usize,
    data: Rc<UnsafeCell<ThrottleData>>,
}

impl Throttler {
    /// Creates a new throttler and sets two timers on reg,
    /// one for updating the tokens, the other for flushing out
    /// blocked peers.
    pub fn new(rate: usize, max_tokens: usize, reg: &Registrar) -> Throttler {
        let id = reg.set_interval(5).unwrap();
        let fid = reg.set_interval(50).unwrap();
        let t = ThrottleData::new(rate, max_tokens);
        Throttler {
            id,
            fid,
            data: Rc::new(UnsafeCell::new(t)),
        }
    }

    pub fn update(&mut self) {
        self.data().add_tokens();
    }

    pub fn get_throttle(&self) -> Throttle {
        Throttle { data: self.data.clone(), id: 0 }
    }

    pub fn set_rate(&mut self, rate: usize) {
        self.data().rate = rate;
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn fid(&self) -> usize {
        self.fid
    }

    pub fn flush(&mut self) -> Vec<usize> {
        self.data().throttled.drain().collect()
    }

    fn data(&self) -> &'static mut ThrottleData {
        unsafe {
            self.data.get().as_mut().unwrap()
        }
    }
}

struct ThrottleData {
    rate: usize,
    tokens: usize,
    max_tokens: usize,
    throttled: HashSet<usize>,
}

/// Throttle mechanism based on the token bucket algorithm.
/// Expected to be called every millisecond, and operates on
/// a KB/s rate scale.
#[derive(Clone)]
pub struct Throttle {
    pub id: usize,
    data: Rc<UnsafeCell<ThrottleData>>,
}

unsafe impl Send for Throttle { }

impl Throttle {
    pub fn get_bytes(&mut self, amnt: usize) -> Result<(), ()> {
        let res = self.data().get_tokens(amnt);
        if res.is_err() {
            self.data().throttled.insert(self.id);
        }
        res
    }

    pub fn restore_bytes(&mut self, amnt: usize) {
        self.data().restore_tokens(amnt);
    }

    fn data(&self) -> &'static mut ThrottleData {
        unsafe {
            self.data.get().as_mut().unwrap()
        }
    }
}

impl Drop for Throttle {
    fn drop(&mut self) {
        self.data().throttled.remove(&self.id);
    }
}

impl ThrottleData {
    /// Creates a new Throttle with the given rate and max token amount.
    fn new(rate: usize, max_tokens: usize) -> ThrottleData {
        ThrottleData { tokens: 0, rate, max_tokens, throttled: HashSet::new(), }
    }

    /// Adds some amount of tokens back.
    fn restore_tokens(&mut self, amnt: usize) {
        self.tokens += amnt;
        if self.tokens >= self.max_tokens {
            self.tokens = self.max_tokens;
        }
    }

    /// This method must be called every 5 milliseconds.
    fn add_tokens(&mut self) {
        self.tokens += self.rate * 5;
        if self.tokens >= self.max_tokens {
            self.tokens = self.max_tokens;
        }
    }

    /// Attempt to extract amnt tokens from the throttler.
    fn get_tokens(&mut self, amnt: usize) -> Result<(), ()> {
        if self.rate == 0 {
            return Ok(())
        }
        if amnt > self.tokens {
            Err(())
        } else {
            self.tokens -= amnt;
            Ok(())
        }
    }
}
