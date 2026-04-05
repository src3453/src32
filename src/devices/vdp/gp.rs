pub struct Gp0 {
    pub vram: Vec<u8>,
}

impl Gp0 {
    pub fn new(size: usize) -> Self {
        Self {
            vram: vec![0; size],
        }
    }

    pub fn get_vram(&self) -> &[u8] {
        &self.vram
    }
}
