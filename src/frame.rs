/// Linear-light working buffer for every rendering stage.
#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub width: usize,
    pub height: usize,
    pixels: Vec<[f32; 3]>,
}

impl Frame {
    /// Build a frame, enforcing the buffer/dimension invariant.
    pub fn new(width: usize, height: usize, pixels: Vec<[f32; 3]>) -> Self {
        assert_eq!(
            pixels.len(),
            width * height,
            "frame buffer must match its dimensions"
        );
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn zeros(width: usize, height: usize) -> Self {
        Self::new(width, height, vec![[0.0; 3]; width * height])
    }

    pub fn pixels(&self) -> &[[f32; 3]] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [[f32; 3]] {
        &mut self.pixels
    }

    pub fn into_parts(self) -> (usize, usize, Vec<[f32; 3]>) {
        (self.width, self.height, self.pixels)
    }
}
