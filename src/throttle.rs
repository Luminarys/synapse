
/// Throttle mechanism based on the token bucket algorithm.
/// Expected to be called every millisecond, and operates on
/// a KB/s rate scale.
pub struct Throttle {
    rate: usize,
    tokens: usize,
    max_tokens: usize,
}

impl Throttle {
    /// Creates a new Throttle with the given rate and max token amount.
    pub fn new(rate: usize, max_tokens: usize) -> Throttle {
        Throttle { tokens: 0, rate, max_tokens }
    }

    /// This method must be called every millisecond.
    pub fn add_tokens(&mut self) {
        self.tokens += self.rate;
        if self.tokens >= self.max_tokens {
            self.tokens = self.max_tokens;
        }
    }

    /// Attempt to extract amnt tokens from the throttler.
    pub fn get_tokens(&mut self, amnt: usize) -> Result<(), ()> {
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
