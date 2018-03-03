use std::cell::RefCell;
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
    dl_data: Rc<RefCell<ThrottleData>>,
    ul_data: Rc<RefCell<ThrottleData>>,
}

const URATE: usize = 15;

impl Throttler {
    /// Creates a new throttler and sets two timers on reg,
    /// one for updating the tokens, the other for flushing out
    /// blocked peers.
    pub fn new(
        dl_rate: Option<i64>,
        ul_rate: Option<i64>,
        max_tokens: usize,
        reg: &Registrar,
    ) -> Throttler {
        let id = reg.set_interval(URATE).unwrap();
        let fid = reg.set_interval(50).unwrap();
        let ut = ThrottleData::new(ul_rate, max_tokens);
        let dt = ThrottleData::new(dl_rate, max_tokens);
        Throttler {
            id,
            fid,
            ul_data: Rc::new(RefCell::new(ut)),
            dl_data: Rc::new(RefCell::new(dt)),
        }
    }

    #[cfg(test)]
    pub fn test(dl_rate: Option<i64>, ul_rate: Option<i64>, max_tokens: usize) -> Throttler {
        let ut = ThrottleData::new(ul_rate, max_tokens);
        let dt = ThrottleData::new(dl_rate, max_tokens);
        Throttler {
            id: 0,
            fid: 0,
            ul_data: Rc::new(RefCell::new(ut)),
            dl_data: Rc::new(RefCell::new(dt)),
        }
    }

    pub fn update(&self) -> (u64, u64) {
        let ul = self.ul_data.borrow_mut().add_tokens();
        let dl = self.dl_data.borrow_mut().add_tokens();
        (ul, dl)
    }

    pub fn get_throttle(&self, id: usize) -> Throttle {
        Throttle {
            ul_data: self.ul_data.clone(),
            ul_tier: Rc::new(RefCell::new(ThrottleData::new(
                None,
                self.ul_data.borrow().max_tokens,
            ))),
            dl_data: self.dl_data.clone(),
            dl_tier: Rc::new(RefCell::new(ThrottleData::new(
                None,
                self.dl_data.borrow().max_tokens,
            ))),
            id,
        }
    }

    pub fn ul_rate(&mut self) -> Option<i64> {
        self.ul_data.borrow().rate
    }

    pub fn dl_rate(&mut self) -> Option<i64> {
        self.dl_data.borrow().rate
    }

    pub fn set_ul_rate(&mut self, rate: Option<i64>) {
        self.ul_data.borrow_mut().rate = rate;
    }

    pub fn set_dl_rate(&mut self, rate: Option<i64>) {
        self.dl_data.borrow_mut().rate = rate;
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn fid(&self) -> usize {
        self.fid
    }

    pub fn flush_ul(&mut self) -> Vec<usize> {
        let mut ul_data = self.ul_data.borrow_mut();
        let flushed = ul_data.throttled.drain().collect();
        flushed
    }

    pub fn flush_dl(&mut self) -> Vec<usize> {
        let mut dl_data = self.dl_data.borrow_mut();
        let flushed = dl_data.throttled.drain().collect();
        flushed
    }
}

struct ThrottleData {
    rate: Option<i64>,
    tokens: usize,
    epoch: usize,
    max_tokens: usize,
    last_used: u64,
    throttled: HashSet<usize>,
}

/// Throttle mechanism based on the token bucket algorithm.
/// Expected to be called every millisecond, and operates on
/// a KB/s rate scale.
#[derive(Clone)]
pub struct Throttle {
    pub id: usize,
    ul_tier: Rc<RefCell<ThrottleData>>,
    dl_tier: Rc<RefCell<ThrottleData>>,
    ul_data: Rc<RefCell<ThrottleData>>,
    dl_data: Rc<RefCell<ThrottleData>>,
}

unsafe impl Send for Throttle {}

impl Throttle {
    pub fn new_sibling(&self, id: usize) -> Throttle {
        Throttle {
            ul_data: self.ul_data.clone(),
            ul_tier: self.ul_tier.clone(),
            dl_data: self.dl_data.clone(),
            dl_tier: self.dl_tier.clone(),
            id,
        }
    }

    pub fn get_bytes_dl(&mut self, amnt: usize) -> Result<(), ()> {
        while self.dl_tier.borrow().epoch != self.dl_data.borrow().epoch {
            self.dl_tier.borrow_mut().add_tokens();
        }
        if self.dl_rate() == Some(-1) {
            self.dl_tier.borrow_mut().last_used += amnt as u64;
            self.dl_data.borrow_mut().last_used += amnt as u64;
            return Ok(());
        }
        let pres = self.dl_data.borrow_mut().get_tokens(amnt);
        if pres.is_err() {
            self.dl_data.borrow_mut().throttled.insert(self.id);
            return Err(());
        }

        let res = self.dl_tier.borrow_mut().get_tokens(amnt);
        if res.is_err() {
            self.dl_data.borrow_mut().restore_tokens(amnt);
            self.dl_data.borrow_mut().throttled.insert(self.id);
            return Err(());
        }
        Ok(())
    }

    pub fn get_bytes_ul(&mut self, amnt: usize) -> Result<(), ()> {
        while self.ul_tier.borrow().epoch != self.ul_data.borrow().epoch {
            self.ul_tier.borrow_mut().add_tokens();
        }
        if self.ul_rate() == Some(-1) {
            self.ul_tier.borrow_mut().last_used += amnt as u64;
            self.ul_data.borrow_mut().last_used += amnt as u64;
            return Ok(());
        }
        let pres = self.ul_data.borrow_mut().get_tokens(amnt);
        if pres.is_err() {
            self.ul_data.borrow_mut().throttled.insert(self.id);
            return Err(());
        }

        let res = self.ul_tier.borrow_mut().get_tokens(amnt);
        if res.is_err() {
            self.ul_data.borrow_mut().restore_tokens(amnt);
            self.ul_data.borrow_mut().throttled.insert(self.id);
            return Err(());
        }
        Ok(())
    }

    pub fn ul_rate(&self) -> Option<i64> {
        self.ul_tier.borrow_mut().rate
    }

    pub fn dl_rate(&self) -> Option<i64> {
        self.dl_tier.borrow_mut().rate
    }

    pub fn set_ul_rate(&mut self, rate: Option<i64>) {
        self.ul_tier.borrow_mut().rate = rate;
    }

    pub fn set_dl_rate(&mut self, rate: Option<i64>) {
        self.dl_tier.borrow_mut().rate = rate;
    }

    pub fn restore_bytes_dl(&mut self, amnt: usize) {
        self.dl_data.borrow_mut().restore_tokens(amnt);
        self.dl_tier.borrow_mut().restore_tokens(amnt);
    }

    pub fn restore_bytes_ul(&mut self, amnt: usize) {
        self.ul_data.borrow_mut().restore_tokens(amnt);
        self.ul_tier.borrow_mut().restore_tokens(amnt);
    }
}

impl Drop for Throttle {
    fn drop(&mut self) {
        self.ul_data.borrow_mut().throttled.remove(&self.id);
        self.dl_data.borrow_mut().throttled.remove(&self.id);
    }
}

impl ThrottleData {
    /// Creates a new Throttle with the given rate and max token amount.
    fn new(rate: Option<i64>, max_tokens: usize) -> ThrottleData {
        ThrottleData {
            tokens: 0,
            rate,
            max_tokens,
            throttled: HashSet::with_capacity(0),
            last_used: 0,
            epoch: 0,
        }
    }

    /// Adds some amount of tokens back.
    fn restore_tokens(&mut self, amnt: usize) {
        self.last_used -= amnt as u64;
        self.tokens += amnt;
    }

    /// This method must be called every URATE milliseconds and returns
    /// (self.last_used) * 1000/URATE - the bits/s, clearing self.last_used
    fn add_tokens(&mut self) -> u64 {
        self.epoch = self.epoch.wrapping_add(1);
        let drained = self.last_used as u64;
        self.last_used = 0;
        self.tokens += if let Some(r) = self.rate {
            if r > 0 {
                (r as usize * URATE) / 1000
            } else {
                0
            }
        } else {
            0
        };
        if self.tokens >= self.max_tokens {
            self.tokens = self.max_tokens;
        }
        drained
    }

    /// Attempt to extract amnt tokens from the throttler.
    fn get_tokens(&mut self, amnt: usize) -> Result<(), ()> {
        match self.rate {
            None => {
                self.last_used += amnt as u64;
                Ok(())
            }
            Some(i) if i < 0 => {
                self.last_used += amnt as u64;
                Ok(())
            }
            Some(_) => {
                if amnt > self.tokens {
                    Err(())
                } else {
                    self.last_used += amnt as u64;
                    self.tokens -= amnt;
                    Ok(())
                }
            }
        }
    }
}
