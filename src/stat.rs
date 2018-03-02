use std::time;

const ALPHA: f64 = 0.8;

#[derive(Debug)]
pub struct EMA {
    ul: u64,
    dl: u64,
    accum_ul: f64,
    accum_dl: f64,
    accum_time: f64,
    updated: time::Instant,
}

impl EMA {
    pub fn new() -> EMA {
        EMA {
            ul: 0,
            dl: 0,
            accum_ul: 0.,
            accum_dl: 0.,
            accum_time: 1.,
            updated: time::Instant::now(),
        }
    }

    pub fn active(&self) -> bool {
        self.accum_ul > 0.1 || self.accum_dl > 0.1
    }

    pub fn avg_ul(&self) -> u64 {
        (1000.0 * self.accum_ul / self.accum_time) as u64
    }

    pub fn avg_dl(&self) -> u64 {
        (1000.0 * self.accum_dl / self.accum_time) as u64
    }

    pub fn add_ul(&mut self, amnt: u64) {
        self.ul += amnt;
    }

    pub fn add_dl(&mut self, amnt: u64) {
        self.dl += amnt;
    }

    pub fn tick(&mut self) {
        self.accum_ul = (ALPHA * self.ul as f64) + (1.0 - ALPHA) * self.accum_ul;
        self.accum_dl = (ALPHA * self.dl as f64) + (1.0 - ALPHA) * self.accum_dl;
        self.ul = 0;
        self.dl = 0;
        // Put everything in terms of milliseconds
        let elapsed = self.updated.elapsed();
        let dur = (elapsed.as_secs() * 1000) as f64 + elapsed.subsec_nanos() as f64 / 1000000.0;
        self.accum_time = (ALPHA * dur) + (1.0 - ALPHA) * self.accum_time;
        self.updated = time::Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_ema() {
        let mut s = EMA::new();
        s.add_ul(1000);
        thread::sleep(time::Duration::from_millis(50));
        s.tick();

        s.add_ul(0);
        thread::sleep(time::Duration::from_millis(50));
        s.tick();

        s.add_ul(500);
        thread::sleep(time::Duration::from_millis(50));
        s.tick();

        assert!((s.avg_ul() as i64 - 10000).abs() < 8000);
    }
}
