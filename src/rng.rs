/// SplitMix64 streams with Box-Muller normal pairs; no global mutable RNG.
pub(crate) struct Rng {
    state: u64,
    spare: Option<f32>,
}

impl Rng {
    pub fn stream(seed: u64, stream: u64) -> Self {
        let mut derived = Self {
            state: seed ^ stream.wrapping_mul(0xd1b54a32d192ed03),
            spare: None,
        };
        Self {
            state: derived.next_u64(),
            spare: None,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^ (value >> 31)
    }

    fn unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / 9_007_199_254_740_992.0
    }

    pub fn uniform(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit() as f32
    }

    pub fn normal(&mut self) -> f32 {
        if let Some(value) = self.spare.take() {
            return value;
        }
        let radius = (-2.0 * self.unit().ln()).sqrt();
        let (sine, cosine) = (std::f64::consts::TAU * self.unit()).sin_cos();
        self.spare = Some((radius * sine) as f32);
        (radius * cosine) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn normal_noise_has_the_requested_unit_variance() {
        let mut rng = Rng::stream(20261107, 2);
        let mut sum = 0.0;
        let mut squares = 0.0;
        for _ in 0..20_000 {
            let value = rng.normal() as f64;
            sum += value;
            squares += value * value;
        }
        let mean = sum / 20_000.0;
        let variance = squares / 20_000.0 - mean * mean;
        assert!(mean.abs() < 0.03);
        assert!((0.95..1.05).contains(&variance));
    }
}
