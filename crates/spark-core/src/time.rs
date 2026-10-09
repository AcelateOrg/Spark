/// Frame timing. All game-facing values are *scaled* by [`Time::scale`] (slow motion / pause).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Time {
    /// Scaled seconds since the previous frame (what `update` receives).
    pub dt: f32,
    /// Real seconds since the previous frame (ignores `scale`).
    pub unscaled_dt: f32,
    /// Scaled seconds since start.
    pub elapsed: f64,
    /// Real seconds since start.
    pub unscaled_elapsed: f64,
    /// Frames since start (1 during the first update).
    pub frame: u64,
    /// Frames per second (averaged over the last second, 0 in headless mode).
    pub fps: f32,
    /// Game speed: 1 = normal, 0.5 = slow motion, 0 = paused. Affects dt, timers, fixed updates.
    pub scale: f32,
    /// Fixed time step for `fixed_update` (physics), seconds. Default 1/60.
    pub fixed_dt: f32,
    /// Max fixed steps per frame (prevents the "spiral of death" after a hitch).
    pub max_fixed_steps: u32,
    /// Leftover scaled time not yet consumed by fixed steps.
    pub fixed_accumulator: f32,
}

impl Default for Time {
    fn default() -> Self {
        Self {
            dt: 0.0,
            unscaled_dt: 0.0,
            elapsed: 0.0,
            unscaled_elapsed: 0.0,
            frame: 0,
            fps: 0.0,
            scale: 1.0,
            fixed_dt: 1.0 / 60.0,
            max_fixed_steps: 8,
            fixed_accumulator: 0.0,
        }
    }
}

impl Time {
    /// Starts a new frame with `real_dt` real seconds.
    pub fn advance(&mut self, real_dt: f32) {
        let real_dt = real_dt.max(0.0);
        self.unscaled_dt = real_dt;
        self.unscaled_elapsed += real_dt as f64;
        self.dt = real_dt * self.scale.max(0.0);
        self.elapsed += self.dt as f64;
        self.frame += 1;
    }

    /// How many `fixed_update` steps to run this frame (consumes the accumulator).
    pub fn fixed_steps(&mut self) -> u32 {
        let step = self.fixed_dt.max(1e-4);
        self.fixed_accumulator += self.dt;
        // Small epsilon so dt == fixed_dt always gives exactly one step despite float error.
        let mut n = ((self.fixed_accumulator + step * 1e-3) / step).floor() as u32;
        if n > self.max_fixed_steps {
            n = self.max_fixed_steps;
            self.fixed_accumulator = 0.0;
        } else {
            self.fixed_accumulator = (self.fixed_accumulator - n as f32 * step).max(0.0);
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_steps_match_frame_rate() {
        let mut t = Time::default();
        let mut total = 0;
        for _ in 0..600 {
            t.advance(1.0 / 60.0);
            total += t.fixed_steps();
        }
        assert_eq!(total, 600);

        let mut t = Time::default();
        let mut total = 0;
        for _ in 0..300 {
            t.advance(1.0 / 30.0);
            total += t.fixed_steps();
        }
        assert_eq!(total, 600);
    }

    #[test]
    fn scale_and_spiral_cap() {
        let mut t = Time::default();
        t.scale = 0.0;
        t.advance(0.5);
        assert_eq!(t.dt, 0.0);
        assert_eq!(t.fixed_steps(), 0);
        t.scale = 1.0;
        t.advance(5.0);
        assert_eq!(t.fixed_steps(), 8);
        assert_eq!(t.fixed_accumulator, 0.0);
    }
}
